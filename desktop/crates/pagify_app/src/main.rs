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

mod canvas;
mod caches;
mod dispatch;
mod mac_open;
mod focus;
mod home;
mod hub;
mod instance;
mod logo;
mod overlay;
mod edit;
mod panels;
mod pending;
mod picking;
#[cfg(test)]
mod edit_text_hardening_tests;
#[cfg(test)]
mod tool_state_gap_tests;
#[cfg(target_os = "windows")]
mod print_windows;
mod ribbon;
mod view;
mod workspace;
mod system_fonts;
mod text_style_panel;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;

use focus::Focus;
use overlay::PageView;
use pagify_shell::automate::{Recorder, Script};
use pagify_shell::block_input::{self, looks_rotated, PageBlocks, PickPath, PickTrace};
use pagify_shell::command::{CommandBox, Dispatch, Escaped, Kind, Submit};
use pagify_shell::markup::{Markup, HIT_TOLERANCE_PT};
use pagify_shell::measure::{self, Calibration};
use pagify_shell::page_space::AppPoint;
use pagify_shell::reader::{prefetch_targets, Strip, PAGE_GAP_PT};
use pagify_shell::recent::Recent;
use pagify_shell::tools::{self, SnapSet};
use pagify_shell::verbs::{self, MeasureKind, PageTarget, SignatureAction, Verb, ZoomTarget};
use pagify_shell::{PageRaster, Session};
use pagify_shell::paragraph_lines::{
    asks_for_width, check_lines_up, commands_for, count_of, ends_at_one_margin, line_edits,
    paragraph_applied_message, plan_log_line, plan_paragraph_edit, recolour_targets, removed_pieces,
    STRETCH_JUSTIFIED_LINES,
};
use pdf_core::document::Color;
use pdf_core::error::PdfError;
use pdf_core::Rotation;
// Re-exported, not just `use`d: `ribbon`'s own sibling modules (the test
// files `use super::*;`, and `hub.rs` reaches these as `crate::X`) need the
// same names visible through the crate root, which only a `pub(crate) use`
// — not a plain one — actually propagates through a glob import.
pub(crate) use ribbon::{
    doc_tab_button, doc_tab_height, doc_tab_width, fit_tab_label, history_toggle, more_tools_button, ribbon_click,
    ribbon_overflow_at, tab_button, tab_menu_button, tabs_that_fit, tool_button, RibbonClick, Tab, DOC_TAB_FONT,
    DOC_TAB_MAX_TEXT, DOC_TAB_MENU_WIDTH, DOC_TAB_PADDING, RIBBON_MARGIN_X, RIBBON_MARGIN_Y, TOOL_HEIGHT, TOOL_WIDTH,
};
pub(crate) use pending::{
    ArmedTool, DrawKind, MatchPropertiesSample, PendingArticleBox, PendingLink, Tool,
};
// `spelling` and `paragraph_lines` moved to `pagify_shell` (Phase 4a: no
// egui, so they belong where they can be tested without a window) —
// re-exported under their old names so every existing `spelling::X` /
// `paragraph_lines::X` call and test import keeps resolving.
pub(crate) use pagify_shell::spelling;
pub(crate) use pagify_shell::paragraph_lines::{
    self, fix_extracted_text, hyphens_before_drawn_lines, is_hyphen_mark, join_paragraph_lines, justify_gaps,
    majority_look, paragraph_should_justify, wrap_hyphen_marks, LineEnd,
};
pub(crate) use pagify_shell::reader::{reveal_axis, REVEAL_AIR_PX, STRIP_PAD_PX};
// The run editor's own arithmetic — size fallbacks, box growth, face
// sectioning, background quantisation — moved to `pagify_shell::editor`
// (design review Phase 4a): all of it is a function of numbers and text, and
// none of it needs a window to be tested. Re-exported under the old names so
// every call and test import keeps resolving.
pub(crate) use pagify_shell::editor::{
    editor_sections, quantize_to_nearest_8, run_editor_box_grow, run_editor_font_size, run_editor_glyph_size,
};

/// Switches a test flips to make part of the app fail on purpose, and the
/// helpers the tests that edit a page share.
#[cfg(test)]
mod tests_support;

const COMMAND_INPUT: &str = "pagify::command_input";
/// About how wide a page is drawn in the Organize grid; the real width is
/// whatever makes a whole number of columns fill the panel.
const GRID_CELL_PT: f32 = 90.0;
/// The narrowest a page is ever drawn in the Organize grid.
const GRID_CELL_MIN_PT: f32 = 48.0;
/// The gap, across and down, between the Organize grid's pages.
const GRID_GAP_PT: f32 = 8.0;
/// The Organize grid's side panel. **Its own id, not the Pages rail's
/// `"thumbs"`**: a panel remembers its width under its id, so with one id
/// each of them opened at whatever width the other had last been left at —
/// and the grid left the rail at whatever the grid had grown to.
const ORGANIZE_GRID_PANEL: &str = "organize_grid";
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

    let mut files: Vec<String> = Vec::new();
    let mut startup: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => {
                if let Some(command) = args.next() {
                    startup.push(command);
                }
            }
            _ => files.push(arg),
        }
    }

    // **One Pagify.** If one is already running, this launch is only a request
    // to it — its files become tabs of the window already open — and exits
    // without a window of its own. See `instance`.
    let running = match pagify_shell::state::state_dir() {
        Some(dir) => instance::start(&dir, &instance::Request::new(&files, &startup), instance::ANSWER_WITHIN),
        None => instance::Start::Independent,
    };
    if matches!(running, instance::Start::Forwarded) {
        return Ok(());
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

            let mut app = PagifyApp::new(files.first().map(String::as_str));
            // Every file named, not just the first: each is a tab.
            for file in files.iter().skip(1) {
                app.open(file);
            }
            for command in &startup {
                app.submit(command);
            }
            // The program is the `Hub`, which owns the windows — the one just
            // made is the first. See `hub`.
            let handover = match running {
                instance::Start::Primary(primary) => instance::Handover::watch(primary, cc.egui_ctx.clone()),
                _ => instance::Handover::default(),
            };
            Ok(Box::new(hub::Hub::new(app, handover)))
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ZoomMode {
    Factor(f32),
    Fit,
    Width,
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

/// What the Search & Replace panel is set to do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum FindReplaceMode {
    /// Search without replacing anything — `find`/`findnext` themselves,
    /// reached from the panel instead of the command box.
    #[default]
    Find,
    /// Replace the current match and step to the next, one at a time —
    /// so a match that should be left alone can be skipped with "Find
    /// Next" instead of being replaced by mistake.
    ReplaceOne,
    /// Replace every match across the whole document at once.
    ReplaceAll,
}

/// The Search & Replace panel, while it is open.
#[derive(Debug, Default, Clone)]
struct FindReplace {
    mode: FindReplaceMode,
    /// What to search for.
    find: String,
    /// What to put in its place.
    replace: String,
}

/// One word the checker does not recognise.
///
/// Keeps the run's own object rather than a page-wide byte offset: applying
/// a fix re-finds `word` inside that run's *current* text at the moment it
/// is applied (the same thing `replace_current` already does), so an
/// earlier fix changing that run's length never invalidates a later one.
#[derive(Debug, Clone, PartialEq)]
struct Misspelling {
    page: usize,
    object: usize,
    word: String,
}

/// The Check Spelling panel, while it is open.
#[derive(Debug, Default, Clone)]
struct SpellCheck {
    /// Every word still to review, in the order the pages were scanned.
    found: Vec<Misspelling>,
    /// What "Change" would write for `found[0]` — the top suggestion by
    /// default, but a person may type something else in its place.
    replacement: String,
    /// Read once, when the panel opens: how many words the scan reported
    /// found in total, kept so "3 of 12" still means something once fixes
    /// and skips have shortened `found` down to fewer than that.
    total_found: usize,
    /// The suggestions already worked out, by the word's lowercase spelling.
    /// A search is hundreds of thousands of edit distances and the panel is
    /// drawn on every mouse move, so each word is searched for once. Keyed by
    /// the word and not by a place in `found`: Change and Ignore shuffle that
    /// list, and a word's suggestions follow the word. Dropped with the panel,
    /// so a re-check starts clean.
    suggestions: HashMap<String, Vec<String>>,
    /// Pages the scan did not check because they are mostly in a script or
    /// language none of the dictionaries can judge (see
    /// `spelling::misspelled_in_page`).
    skipped_pages: Vec<usize>,
    /// Whether any page has Chinese on it, which is only checked for unusual
    /// characters — the panel says so.
    chinese: bool,
    /// Why the last Change did not go through, shown in the panel until the
    /// next action. The word stays in `found` while this is set.
    notice: Option<String>,
    /// `Some((pages checked, pages in all))` while the scan is still running on
    /// its own thread. The panel shows a spinner and the count instead of a word.
    scanning: Option<(usize, usize)>,
}

/// A spelling scan running on its own thread.
///
/// **Reported from use: "spell check freezes".** The scan read the text of every
/// page, and checked every word on it, on the thread that draws the window — a
/// document of a few hundred pages stopped responding until it was done. Now it
/// runs beside the window, as the OCR reader does: the window stays alive, shows
/// how far it has got, and the scan is put down (not finished for nobody) when
/// the panel is closed or this is dropped with its tab.
struct SpellScan {
    done: std::sync::mpsc::Receiver<ScanMessage>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pages: usize,
}

impl Drop for SpellScan {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

enum ScanMessage {
    /// Pages looked at so far.
    Progress(usize),
    Finished { found: Vec<Misspelling>, skipped: Vec<usize>, chinese: bool },
}

impl SpellCheck {
    /// The suggestions for `word`, searched for the first time it is asked.
    fn suggestions_for(&mut self, word: &str) -> &[String] {
        self.suggestions
            .entry(word.to_lowercase())
            .or_insert_with(|| spelling::suggest(word, 5))
    }

    /// Put what "Change" would write back to the best suggestion for
    /// whichever word is now first (nothing when none is left).
    fn reset_replacement(&mut self) {
        self.replacement = match self.found.first().map(|m| m.word.clone()) {
            Some(word) => self.suggestions_for(&word).first().cloned().unwrap_or_default(),
            None => String::new(),
        };
    }

    /// One sentence saying which pages were skipped, or `None` when none was.
    fn skipped_note(&self) -> Option<String> {
        let numbers: Vec<String> = self.skipped_pages.iter().map(|p| (p + 1).to_string()).collect();
        let (who, were) = match numbers.as_slice() {
            [] => return None,
            [one] => (format!("Page {one} is"), "it was"),
            [a, b] => (format!("Pages {a} and {b} are"), "they were"),
            [a, b, c] => (format!("Pages {a}, {b} and {c} are"), "they were"),
            many => (format!("{} pages are", many.len()), "they were"),
        };
        Some(format!(
            "{who} mostly in a language the spelling check has no dictionary for - {were} skipped."
        ))
    }

    /// What the check of Chinese amounts to, when there was any.
    fn chinese_note(&self) -> Option<&'static str> {
        self.chinese.then_some(
            "Chinese is checked for unusual characters only - a wrong but real character cannot be found.",
        )
    }
}


/// The Extract dialog while it is open: the pages being asked for, and what was
/// wrong with them the last time Extract was pressed.
///
/// **Reported from use (report 10): "Extract lacks a UI".** The button only put
/// `extract ` in the command box and waited for a page range *and* a path to be
/// typed. This is the same command with the two things it needs asked for in
/// turn — the pages here, the file in the system's own Save box — and it ends
/// by running `extract <pages> <path>`, so the typed form is untouched.
#[derive(Debug, Clone, Default)]
struct ExtractAsk {
    pages: String,
    problem: Option<String>,
    focused: bool,
}

/// `[0, 1, 2, 6]` → `"1-3,7"`: zero-based indices as the one-based range the
/// page-taking commands read.
fn compact_page_spec(pages: &[usize]) -> String {
    let mut sorted = pages.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts = Vec::new();
    let mut start = 0;
    while start < sorted.len() {
        let mut end = start;
        while end + 1 < sorted.len() && sorted[end + 1] == sorted[end] + 1 {
            end += 1;
        }
        parts.push(if end == start {
            (sorted[start] + 1).to_string()
        } else {
            format!("{}-{}", sorted[start] + 1, sorted[end] + 1)
        });
        start = end + 1;
    }
    parts.join(",")
}



/// The Bookmarks panel — every bookmark in the document's own outline,
/// title and page, click to jump.
///
/// Refreshed whenever it is opened or a bookmark is added, rather than read
/// live every frame: the list only ever changes on those two occasions, and
/// re-reading PDFium's own outline tree on every repaint would be work with
/// no payoff.
#[derive(Debug, Default, Clone)]
struct BookmarkPanel {
    entries: Vec<(String, usize)>,
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



struct Doc {
    /// Shared, so recognition can run off the UI thread.
    ///
    /// `Session` is just a handle; every call through it takes the registry
    /// lock, which is what makes PDFium access from two threads safe at all.
    /// The `Arc` is about *lifetime* — the worker must not outlive the document
    /// it is reading — not about safety.
    session: std::sync::Arc<Session>,
    /// Which document this is, for a render that comes back from the worker
    /// after the tab it was asked for may have gone.
    id: u64,
    /// Moved by every [`Doc::rendered_is_stale`], so a render started before an
    /// edit can be told from one started after it.
    render_epoch: u64,
    strip: Strip,
    page_count: usize,
    /// Everything about this document's pages that is cheaper to keep than
    /// to ask the engine for again — see [`caches::DocCaches`].
    caches: caches::DocCaches,
}

/// A sharper render of one crop of a page, cached the same way the
/// whole-page textures are: held until the viewport moves far enough, or the
/// zoom changes enough, that it stops covering what's actually on screen.
struct DetailTile {
    page: usize,
    /// The quantised zoom step this was rendered at — see
    /// [`pdf_core::render::cache::quantise_zoom`]. Re-rendered whenever this
    /// no longer matches, same as a whole-page texture would be.
    zoom_step: u32,
    /// The crop this tile covers, in page points — wider than the viewport by
    /// [`PagifyApp::DETAIL_MARGIN`] on every side, so a small pan or scroll
    /// does not immediately invalidate it.
    crop: pdf_core::document::Rect,
    texture: egui::TextureHandle,
}

/// A word to bring into view: the page it is on and its box on that page, in
/// page points. Asked for by Find and resolved by the next `draw_pages`, which
/// is the one place that knows the zoom and the window it will be drawn in.
#[derive(Debug, Clone, Copy)]
struct Reveal {
    page: usize,
    rect: pagify_shell::reader::Rect,
}



/// Everything about one open document — its own tab. Fully independent of
/// every other tab: its own page, zoom, scroll position, selections, armed
/// tools, open panels, and undo-adjacent bookkeeping. Reached through
/// [`PagifyApp::tab`]/[`PagifyApp::tab_mut`] rather than held directly, so a
/// method written before tabs existed keeps its own shape — only the access
/// path in front of a field changed.
struct DocTab {
    doc: Option<Doc>,
    markup: Markup,
    calibration: Calibration,
    /// The tool waiting for clicks, if any, and what it has collected so far —
    /// every kind, in one state. See [`ArmedTool`] and [`Tool`].
    tool: Option<ArmedTool>,

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
    /// Where the reader was looking at the end of the last frame, so a change of
    /// zoom, window or pages puts them back at the same place on the page and
    /// not at the same pixel offset.
    view: Option<pagify_shell::reader::ViewSnapshot>,
    /// Set when a zoom needs the scroll offset moved with it, applied on the
    /// next frame's `ScrollArea`.
    anchor_offset: Option<egui::Vec2>,
    /// The one paragraph, if any, open for retyping **on the page**, where it
    /// sits — set by a single click (`pick_text_run`/`pick_paragraph`).
    ///
    /// Not in the command box. Editing a word is a thing you do to the word,
    /// and looking somewhere else to do it means holding the page in your head
    /// while you type — which is exactly what an editor should not ask of you.
    editing_run: Option<EditingRun>,
    /// A brand new run being composed — see [`NewTextBox`]. Never live at
    /// the same time as `editing_run`: `addtext` clears the other selection
    /// mechanisms the same way every other tool does when it is armed.
    new_text_box: Option<NewTextBox>,
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
    /// Waiting for a text selection to link — the same shape as
    /// `markup_armed`, kept as its own field rather than folded into it
    /// because linking needs an extra step (the address) `mark_selection`
    /// has no use for, and adds one anyway.
    link_armed: bool,
    /// A text selection, once made, waiting for the address to link it to.
    pending_link: Option<PendingLink>,
    /// The Extract dialog, while it is open on this tab.
    extract_ask: Option<ExtractAsk>,
    /// Waiting for a text selection to serve as Match Properties' own
    /// sample — the same shape `link_armed` is, and for the same reason:
    /// once the sample is in hand every further selection is matched to it
    /// at once, which `mark_selection`'s own one-shot shape has no use for.
    match_properties_armed: bool,
    /// The sample Match Properties is copying from, once picked. While this
    /// is held, completing another text selection retypes it to match
    /// rather than arming the tool over again — the same "stays in hand"
    /// shape `markup_armed` already has, so a whole page's worth of
    /// mismatched runs can be fixed one selection after another without
    /// going back to the ribbon.
    match_properties_sample: Option<MatchPropertiesSample>,
    /// A drawn Article Box rectangle, waiting for its title.
    pending_article_box: Option<PendingArticleBox>,
    /// The Bookmarks panel, while it is open.
    bookmark_panel: Option<BookmarkPanel>,
    /// Every page this document's own outline points at — read once and
    /// kept current rather than re-walked every frame, so the small icon
    /// `draw_pages` paints in a bookmarked page's corner costs a `HashSet`
    /// lookup per visible page, not a fresh PDFium outline walk per page
    /// per repaint.
    bookmarked_pages: std::collections::HashSet<usize>,
    /// Runs a person has explicitly declared one paragraph, overriding
    /// whatever `pagify_shell::blocks::detect`'s own geometry would find —
    /// `(page, object numbers, top to bottom)`. Session-only: nothing is
    /// written to the page until an edit is actually applied through it, so
    /// there is nothing here for a save to carry and nothing a reopen needs
    /// to restore.
    joined_groups: Vec<JoinedGroup>,
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
    /// **The page "Fit" and "Width" are worked out for.** Not the page being
    /// scrolled past: that one follows the scroll, so a zoom taken from it is
    /// an input to its own decision. With pages of different sizes the page
    /// that fills the window set the zoom, the zoom decided which page fills
    /// the window, and the view flipped between the two for ever with no input
    /// at all (reported from use as "jumping around and glitches" on files of
    /// different page dimensions). It changes only when the reader says so:
    /// opening, going to a page, or choosing Fit / Width.
    zoom_basis: usize,
    /// The zoom drawn last frame, and when it last *changed* (egui's own clock,
    /// in seconds) — what tells a zoom still moving from one that has stopped.
    /// See [`ZOOM_SETTLE_SECS`].
    last_drawn_zoom: f32,
    zoom_changed_at: f64,
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

    /// The image a click landed on, and the page it is on.
    ///
    /// Held so a right-click can offer to lock it: the menu opens on a later
    /// frame than the click that selected it, so what was under the pointer has
    /// to survive in between.
    selected_image: Option<(usize, pdf_core::document::PageImage)>,

    text_selection: Option<std::ops::Range<usize>>,
    /// Where a text drag began. `None` means a drag is selecting marks instead.
    text_drag: Option<AppPoint>,

    find_needle: String,
    /// Every match, as (page, character range).
    find_hits: Vec<(usize, std::ops::Range<usize>)>,
    find_at: usize,
    /// Where the current match is, waiting for the next frame to scroll to it.
    ///
    /// **A page to go to is not a word to show.** Find used to ask for the
    /// page only, so on any page taller than the window the highlighted word
    /// could be below the fold — reported from use as "it jumps to the page but
    /// the match is not in view".
    reveal: Option<Reveal>,
    /// The markup revision at the last successful save. Anything above it is
    /// work that closing would throw away.
    saved_revision: u64,
    /// Where the last right-click landed, kept for the menu built after it.
    right_clicked_at: Option<(usize, AppPoint)>,
    /// What the right-click menu's Join/Match-font/Split actions found,
    /// computed once at the moment of the click.
    ///
    /// **Not recomputed by the menu itself.** Each of these reads every run
    /// on the page — see `compute_right_click_text_actions`. The context
    /// menu closure runs on every repaint of an open popup —
    /// egui keeps redrawing it while it sits there — so computing this
    /// inline the way `link_here` does its own cheap, already-cached lookup
    /// meant a few-hundred-run real document redid a full-page text
    /// extraction several times over on every single frame the menu stayed
    /// open. Reported from use as the app freezing on right-click. Filled
    /// in alongside `right_clicked_at` and simply read from here below,
    /// the same "compute once at the click, read many times after" shape
    /// `selected_image` already used for the image-lock menu.
    right_click_text_actions: Option<RightClickTextActions>,
    /// The object tool, when it is in hand: `true` picks pictures before
    /// words under the pointer, `false` the other way round.
    ///
    /// **A selection tool, not a two-click move.** Asked for from use: a click
    /// selects; the selected thing is moved by holding and dragging it, and
    /// resized by dragging one of the handles on its outline. Nothing changes
    /// in the document until the pointer is let go.
    object_tool: Option<bool>,
    /// What the object tool has selected.
    selected: Option<Selected>,
    /// A drag in progress on the selection — where it started, what part of
    /// the selection was grabbed, and how far it has come.
    grab: Option<Grab>,
    /// The handle under the pointer as of the last frame it was only
    /// *hovering* — read by [`Self::interact_objects`] when a drag starts,
    /// instead of hit-testing the drag's own current position. Same reason
    /// as [`Self::signature_hover_handle`]: `egui` only decides a press has
    /// become a drag once the pointer has moved a few pixels, and by then an
    /// eight-pixel resize handle is already behind it — a fresh hit-test at
    /// that point reads a corner-aimed resize as a body drag instead. This
    /// tool shared `Handle`/`Grab` with the signature one but not the fix,
    /// so an ordinary picture's handles were exactly this bug, unfixed.
    object_hover_handle: Option<Handle>,
    /// Whether Shift was held on the last frame of a turn: the angle is then
    /// snapped to 15 degrees, in what is drawn and in what is applied.
    rotate_snap: bool,
    /// More than one thing, picked up together by dragging a rectangle over
    /// empty page area — see [`Self::interact_objects`]. Moves and deletes
    /// as a group; resizing stays a [`Self::selected`]-only, one-thing-at-a-
    /// time action, since "resize forty characters together" has no one
    /// obvious meaning the way "move them all by the same amount" does.
    /// Cleared by anything that sets [`Self::selected`], and vice versa —
    /// the object tool always has at most one of the two.
    group: Vec<Selected>,
    /// A rectangle being dragged out to build [`Self::group`]: where the
    /// drag started, and where the pointer is now. Only the two corners —
    /// which page objects fall inside it is worked out once, when the drag
    /// ends, not recomputed every frame while it is still being dragged.
    marquee: Option<(AppPoint, AppPoint)>,
    /// A drag moving every member of [`Self::group`] by the same amount —
    /// same shape as [`Self::grab`], kept separate because a group drag has
    /// no handle and no single anchor rect to measure against.
    group_grab: Option<Grab>,
    /// A placed-but-unapplied picture signature, picked with no tool
    /// armed — a click on the signature itself, not the object tool, which
    /// only ever sees page *content* and a signature is deliberately not
    /// that until it is applied. Move and resize share `Handle`/`Grab` with
    /// the object tool's own selection; nothing else does.
    signature_selected: Option<SignatureSelected>,
    /// A drag in progress on [`Self::signature_selected`] — same shape and
    /// same meaning as [`Self::grab`], kept separate because the two
    /// selections are independent and a signature is never page content.
    signature_grab: Option<Grab>,
    /// The handle under the pointer as of the last frame it was only
    /// *hovering* — read by [`Self::interact_signatures`] when a drag
    /// starts, instead of hit-testing the drag's own current position.
    ///
    /// **Why this is not the same thing.** `egui` only decides a press has
    /// become a drag once the pointer has moved past its own threshold, and
    /// `drag_started()` reports the position *by then* — already a few
    /// pixels off from wherever the button actually went down. Against an
    /// eight-pixel handle that is enough to miss it entirely: a resize
    /// aimed precisely at a corner was silently read as a body drag
    /// instead, because by the time the drag was recognised the reported
    /// point had already slid past the handle and onto the rect it sits
    /// on. This is what was under the pointer just before that slide.
    signature_hover_handle: Option<Handle>,
    /// The same mechanism as [`Self::signature_selected`], for a plain
    /// placed picture rather than one carrying a signature's name — see
    /// [`PlacedImageSelected`].
    placed_image_selected: Option<PlacedImageSelected>,
    /// A drag in progress on [`Self::placed_image_selected`] — see
    /// [`Self::signature_grab`].
    placed_image_grab: Option<Grab>,
    /// See [`Self::signature_hover_handle`].
    placed_image_hover_handle: Option<Handle>,
    /// The opacity slider's value while it is being dragged, before it is
    /// applied on release.
    opacity_draft: Option<f32>,
    /// Which entry in that list is picked, **by position in the list**.
    ///
    /// Not by object number: what a group draws is listed under the group's
    /// own number, so several entries share one and a pick keyed by it would
    /// select all of them at once.
    picked_layer: Option<usize>,
    /// Which ribbon tab this document was left on — kept per document so
    /// switching tabs restores exactly how you left it, not just its page.
    ribbon: Tab,

    /// A page being read, off the UI thread.
    ///
    /// Recognition is about a second of solid CPU per page. Run in `update` it
    /// stops the window dead — no repaint, no scrolling, no way to cancel —
    /// and on a twenty-page document that is twenty seconds of a frozen app.
    reading: Option<Reading>,
    /// What the user asked to do, held while they decide what to do about
    /// unsaved marks.
    closing: Option<Closing>,
    /// A save that would write a *first* password over the only unsecured
    /// copy, waiting on the person to say what they meant.
    ///
    /// **A question, not a refusal.** This used to say "use `saveas`" and
    /// stop, which left somebody who had pressed Secure Document unable to
    /// save at all — reported from use as "why can't I just save". The
    /// irreversible thing still needs a deliberate click; it is just one of
    /// the buttons now, beside the two safe ones.
    asking_to_secure: Option<PathBuf>,
    /// Set for the one save that has been told, in the dialog above, to write
    /// the password over the original after all.
    secure_in_place_confirmed: bool,
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
    /// The passcode this document's locks are already made under.
    ///
    /// **A deliberate exception to the line above, and worth being exact about
    /// where it stops.** Reported from use: locking six things meant typing the
    /// same passcode six times, and being asked again for the seventh. Every
    /// lock in a document shares one vault and therefore one passcode, so after
    /// the first the program is asking a question it already knows the answer
    /// to.
    ///
    /// What it is used for is **locking only** — putting more content away
    /// under a passcode already in force. Unlocking still asks, every time,
    /// because that is the direction that reveals something: somebody who walks
    /// up to an unattended screen must not be able to click a padlock open.
    ///
    /// It is held in memory, never written anywhere, wiped when it is dropped,
    /// and let go the moment the document does — see [`Self::forget_passcode`].
    held_passcode: Option<zeroize::Zeroizing<String>>,
    /// What has been typed into the password window.
    /// What is being typed into the password window. Wiped when it is taken
    /// or cleared, not merely emptied: `String::clear` leaves the bytes in the
    /// buffer, and a password that was typed a minute ago should not still be
    /// in memory now. Found by audit.
    password_typed: zeroize::Zeroizing<String>,
    /// Whether the window's field has been given the caret yet.
    password_field_focused: bool,
    /// What the window should say went wrong, if anything.
    password_problem: Option<String>,
    /// Whether the password being chosen is Pagify's own rather than PDF's.
    ///
    /// Held here rather than on the `Awaiting` because it is a toggle in the
    /// window, changed while the same question is being asked.
    password_plus: bool,
    /// The Search & Replace panel, while it is open.
    find_replace: Option<FindReplace>,
    /// The Check Spelling panel, while it is open.
    spelling: Option<SpellCheck>,
    /// The scan feeding [`Self::spelling`], while it is still running.
    spell_scan: Option<SpellScan>,
    /// What a drag on the page means. Set by Hand and Select.
    pointer: pagify_shell::verbs::PointerMode,
    /// **Requested from use: Hand should look like the tool in hand when a
    /// document first opens.** `pointer` itself stays `Select` by default —
    /// dragging over text still selects it immediately, with no tool to pick
    /// first, which several existing behaviours and tests depend on — this
    /// only affects which ribbon button *reads* as pressed, until the first
    /// real ribbon click settles it one way or the other.
    hand_shown_before_any_tool_is_picked: bool,
    drag_from: Option<AppPoint>,
    /// A drag in progress on the markup layer's own selection — see
    /// [`Self::finish_markup_grab`]. Separate from [`Self::grab`] and
    /// [`Self::signature_grab`]: a drawn shape is neither page content nor
    /// an annotation, and moving one commits through `tools::move_selection`
    /// rather than either of those two paths.
    markup_grab: Option<Grab>,
    last_snap: Option<tools::Snapped>,
    /// Which of the two separate undo stacks — the current page's markup
    /// layer, or the document's own command history — most recently changed,
    /// tracked by polling both of their own monotonic edit counters once a
    /// frame. See [`Self::track_undo_recency`] and [`Self::undo_redo`].
    ///
    /// **Reported from use: a shape drawn earlier got undone instead of a
    /// text box just added.** `undo_redo` used to try the markup layer
    /// first, always, regardless of which of the two had actually just
    /// changed — right every time the layer was untouched, and wrong the
    /// moment a page carried both a drawn shape and an edited or newly
    /// placed piece of text.
    prefer_layer_undo: bool,
    last_layer_edits: u64,
    last_doc_generation: u64,
    /// Pages selected in the Organize grid, this tab's own — a page's index
    /// means nothing on another tab's document, so unlike [`PagifyApp::
    /// organize_open`] (one workspace-wide panel toggle, the same convention
    /// `show_thumbs` already uses) this lives per tab.
    organize_selected: Vec<usize>,
    /// The page a plain click last landed on, the anchor a shift-click range
    /// extends from — the standard file-manager range-select idiom. Not
    /// modelled on this file's existing shift-click pattern for canvas
    /// objects ([`Self::extend_selection_at`]), which only toggles one item
    /// at a time: a canvas selection has no natural order to span, a page
    /// grid does.
    organize_anchor: Option<usize>,
    /// A drag-to-reorder in progress in the Organize grid.
    organize_drag: Option<OrganizeDrag>,
}

/// A drag-to-reorder in progress in the Organize grid — which pages are
/// being carried, and where the drag started, so a release that never moved
/// past [`PagifyApp::MIN_DRAG_PX`] reads as the plain click it was.
struct OrganizeDrag {
    moving: Vec<usize>,
    pointer_started_at: egui::Pos2,
}

impl DocTab {
    /// A blank tab — no document, defaults everywhere. [`PagifyApp::open`]
    /// fills in `doc` (and everything downstream of it) once a file is
    /// actually read.
    fn new() -> Self {
        DocTab {
            doc: None,
            markup: Markup::default(),
            calibration: Calibration::default(),
            tool: None,
            page: 0,
            zoom: ZoomMode::Fit,
            rotation: Rotation::None,
            scroll_pt: 0.0,
            last_view: None,
            scroll_offset: egui::Vec2::ZERO,
            view: None,
            anchor_offset: None,
            editing_run: None,
            new_text_box: None,
            next_text_id: 0x0100_0000,
            markup_armed: None,
            link_armed: false,
            pending_link: None,
            extract_ask: None,
            match_properties_armed: false,
            match_properties_sample: None,
            pending_article_box: None,
            bookmark_panel: None,
            bookmarked_pages: std::collections::HashSet::new(),
            joined_groups: Vec::new(),
            selection_page: 0,
            settling: 0,
            zoom_basis: 0,
            last_drawn_zoom: 0.0,
            zoom_changed_at: f64::NEG_INFINITY,
            hover_view: None,
            viewport_rect: None,
            copy_wanted: false,
            pan_by: None,
            scroll_to_pt: None,
            canvas_pt: egui::vec2(800.0, 600.0),
            selected_image: None,
            text_selection: None,
            text_drag: None,
            find_needle: String::new(),
            find_hits: Vec::new(),
            find_at: 0,
            reveal: None,
            saved_revision: 0,
            picked_layer: None,
            right_clicked_at: None,
            right_click_text_actions: None,
            object_tool: None,
            selected: None,
            grab: None,
            object_hover_handle: None,
            rotate_snap: false,
            group: Vec::new(),
            marquee: None,
            group_grab: None,
            signature_selected: None,
            signature_grab: None,
            signature_hover_handle: None,
            placed_image_selected: None,
            placed_image_grab: None,
            placed_image_hover_handle: None,
            opacity_draft: None,
            ribbon: Tab::Home,
            closing: None,
            asking_to_secure: None,
            secure_in_place_confirmed: false,
            asking_to_redact: None,
            reading: None,
            awaiting_password: None,
            held_passcode: None,
            password_typed: zeroize::Zeroizing::new(String::new()),
            password_field_focused: false,
            password_problem: None,
            password_plus: false,
            find_replace: None,
            spelling: None,
            spell_scan: None,
            pointer: Default::default(),
            hand_shown_before_any_tool_is_picked: true,
            drag_from: None,
            markup_grab: None,
            last_snap: None,
            prefer_layer_undo: false,
            last_layer_edits: 0,
            last_doc_generation: 0,
            organize_selected: Vec::new(),
            organize_anchor: None,
            organize_drag: None,
        }
    }
}

struct PagifyApp {
    /// Every open document, in the order its tab sits — see [`DocTab`].
    tabs: Vec<DocTab>,
    /// Which of `tabs` is showing. Always a valid index into `tabs`, which
    /// is never empty while the app is running — closing the last tab quits
    /// instead of leaving this dangling.
    active_tab: usize,

    cmd: CommandBox,
    /// Shared across every tab — recording a macro follows what you actually
    /// did, tab switches included, rather than one silently-incomplete
    /// recording per document.
    recorder: Recorder,

    recent: Recent,

    /// Extra fonts a reader has added for outlined-text recognition, beyond
    /// `BUNDLED_OUTLINED_FONTS` — see `outlined_font_bytes`. A library, like
    /// the signature and predefined-text ones below: added once, available
    /// to every tab.
    outlined_fonts: pagify_shell::outlined_fonts::OutlinedFonts,

    defaults: tools::Defaults,
    /// Whether the next Rectangle or Circle is drawn filled solid rather than
    /// hollow — chosen up front, on the Draw tab, before either tool is armed.
    draw_fill: bool,
    /// Decoded on the first frame — a `Context` is needed to upload it and
    /// there is none when the app is constructed.
    mark: Option<egui::TextureHandle>,
    /// An uploaded signature's picture, ready to paint in the Manage
    /// Signatures list — keyed by name, built the first time that entry is
    /// drawn. A stale entry left behind by a rename or a forgotten signature
    /// costs a little memory and nothing else; the list is never large
    /// enough for that to matter.
    signature_textures: std::collections::HashMap<String, egui::TextureHandle>,
    snaps: SnapSet,
    ortho: bool,
    grid_pt: f64,
    show_thumbs: bool,
    /// Whether the Organize grid is showing in place of the plain thumbnail
    /// rail — workspace-wide, the same convention `show_thumbs` itself
    /// already uses, not per document: the two are mutually exclusive views
    /// of whichever tab is active, not a per-tab mode.
    organize_open: bool,
    /// Whether the layer rail is showing.
    show_layers: bool,
    /// Whether the command box shows its history, or is the single line the
    /// mockup draws. Collapsed by default — the history is worth seeing when
    /// you are working in it and is dead space when you are reading.
    command_open: bool,
    /// How many errors have been said, ever — a counter, not the history,
    /// because the history is capped and its length stops moving. Lets a caller
    /// ask "did that gesture end in an error?" without every function it calls
    /// having to hand a result back (see the click-away block in `interact`).
    errors_said: u64,

    /// Loaded on first use and kept. The models are twelve megabytes and take
    /// a moment to memory-map; doing that per page would make the second page
    /// as slow as the first for no reason — and every tab's own OCR job
    /// shares this one loaded copy rather than each paying to load it again.
    recogniser: Option<std::sync::Arc<pdf_core::ocr::engine::OcrsRecogniser>>,
    /// The signatures this person has drawn, and where they are kept.
    ///
    /// The path is held rather than asked for each time so a test can point it
    /// at a scratch file: writing a test signature into somebody's real
    /// settings would be a poor way to find out this works.
    signatures: pagify_shell::signatures::Signatures,
    signatures_path: Option<std::path::PathBuf>,
    /// Where recorded scripts are written: Pagify's own folder, and nowhere
    /// in a test.
    scripts_dir: Option<std::path::PathBuf>,
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
    /// The real ascent/descent of whatever font `editor_face` names, read
    /// once when it is loaded — see [`Self::want_document_face`]'s own doc
    /// and [`run_editor_font_size`], which sizes the editor from this
    /// instead of a fixed guess when a run's own reported size cannot be
    /// trusted.
    editor_face_metrics: Option<pdf_core::pdf::embed::Metrics>,
    /// Which characters `editor_face` has *ink* for, read once beside the
    /// metrics — see [`pdf_core::pdf::embed::outlined_chars`]. The editor
    /// lays out any other character in the program's own face
    /// (`editor_sections`), since the document's subset would draw it blank.
    /// Shared, not copied, into the layouter every frame.
    editor_face_coverage: Option<std::sync::Arc<pdf_core::pdf::embed::OutlineCoverage>>,
    /// A face asked for and not yet installed, handed to `install_fonts` at the
    /// top of the next frame.
    pending_face: Option<Vec<u8>>,
    /// Words somebody typed that an editor is being let go of without applying —
    /// the engine refused them, or the page changed under the box — to be put on
    /// the clipboard, so that the typing is never simply lost. Said at the next
    /// frame, in `draw_properties_panel`: putting text on the clipboard takes the
    /// `egui::Context` that the apply, which has none, cannot reach.
    text_to_offer: Option<String>,
    /// The font programs the editor has been handed, kept so that a click on a
    /// paragraph the document already showed a face for does not ask the engine
    /// for it again — see [`Self::want_document_face_for`].
    face_cache: FaceCache,
    /// Every font Windows has installed, named and by file path — the run
    /// editor's font picker reads from this rather than scanning the
    /// filesystem itself. `None` until the picker is opened for the first
    /// time: populated lazily because the scan reads and parses every
    /// installed font file, a real cost worth paying once, not on every
    /// frame a document happens to be open.
    system_fonts: Option<Vec<system_fonts::SystemFont>>,
    /// Whether the run editor's font-picker popup is open, and what has been
    /// typed into its filter box.
    font_picker_open: bool,
    font_picker_filter: String,
    /// This run's on-disk transcript of every command and every line the app
    /// has said about it — see [`pagify_shell::session_log`]. One continuous
    /// transcript for the whole run, tabs included, not one per document —
    /// not the same thing as [`Self::recorder`] (Automate): that keeps only
    /// what could be typed back in and replayed; this keeps the outcomes
    /// too, almost none of which are typeable, so a bug can be reproduced
    /// once and the file handed over instead of described from memory.
    session_log: pagify_shell::session_log::SessionLog,
    /// What `copy` last took from a drawn shape or placed picture selection,
    /// for `paste` to lay back down — see [`ObjectClipboard`]. Shared across
    /// tabs on purpose: copying a stamp from one open document and pasting
    /// it into another is a feature, not a leak, and the system clipboard
    /// works the same way for the text case
    /// ([`Self::copy_selection`]/⌘C). Not the system clipboard itself —
    /// these are structured page objects, not text.
    object_clipboard: Option<ObjectClipboard>,
    /// A paste picked up with ⌘V in Edit Object / Edit Text and not yet put
    /// down: drawn under the pointer at 50% opacity, placed by the next click
    /// on a page, dropped by Escape. Shared across tabs like the clipboard.
    paste_ghost: Option<PasteGhost>,
    /// How many times `paste` has run since the clipboard was last filled —
    /// see [`Self::PASTE_STEP`].
    paste_count: u32,
    /// What Organize's own copy last took — a temp file holding exactly the
    /// copied pages, extracted live from whichever tab they were copied on
    /// (via [`pagify_shell::Session::extract_to`], which reads the
    /// document's current in-memory state, not what was last saved to disk —
    /// so copying from a tab with unsaved edits still copies what is on
    /// screen). Shared across tabs, same reasoning as [`Self::object_clipboard`]
    /// right above: pasting pages from one open document into another is a
    /// feature. Deleted and replaced wholesale on every new copy — see
    /// [`Self::copy_organize_selection`] — rather than accumulated, since a
    /// clipboard only ever needs to hold the most recent copy.
    page_clipboard: Option<PageClipboard>,
    /// Where copied pages are left so any Pagify window can paste them — see
    /// [`Self::copy_organize_selection`].
    clipboard_dir: std::path::PathBuf,
    /// **Reported from use**: ⌘V did nothing. egui-winit only emits
    /// `Event::Paste` when the *system* clipboard already holds non-empty
    /// text — unrelated to whether Pagify's own [`Self::page_clipboard`]/
    /// [`Self::object_clipboard`] has anything, so a ⌘C that only ever wrote
    /// to those left the system clipboard untouched and the next ⌘V's
    /// `Event::Paste` never fired at all. Set by [`Self::copy_organize_selection`]/
    /// [`Self::copy_object_selection`] on success; consumed once per frame in
    /// `ui()`, the one place that actually has the `egui::Context` a real
    /// OS-clipboard write needs (neither of those two functions do, and
    /// adding it would mean every test calling them directly would need one
    /// too).
    clipboard_mirror_wanted: bool,
    /// The thread that renders pages off the UI thread, started on first use —
    /// see [`RenderWorker`].
    renders: Option<RenderWorker>,
    /// Whether pages are rendered off the UI thread at all. On everywhere but
    /// in the tests, where a render that lands on some later frame would make
    /// every test that looks at what was drawn depend on timing; the tests that
    /// are about this turn it on.
    async_render: bool,
    render_stats: RenderStats,
    /// Whether this window is *the* running Pagify on its own, answering
    /// documents other launches hand over — and which window a handed-over
    /// document goes to. Not the running instance (the default) answers nothing.
    /// In the program itself that is the `hub::Hub`'s to do, for all its windows;
    /// this is for a window run on its own, which is what the tests do. See
    /// [`instance`].
    handover: instance::Handover,
    /// What makes this one of several windows: whether it is leaving, a tab
    /// being dragged out of it, what other windows are asking of it. Idle in a
    /// program with one window, and in every test. See [`hub`].
    win: hub::WindowState,
    /// How many `replay`s are on the stack right now.
    ///
    /// A script that names itself — or two that name each other — recurses
    /// with nothing else to stop it; `replay`'s own guard against a bad step
    /// only catches a step that fails to *dispatch*, and a further `replay`
    /// dispatches just fine. Found by audit.
    replay_depth: usize,
    /// A check for a newer build in [`UPDATE_FOLDER`], in progress — see
    /// [`Self::collect_update_check`]. `None` once it has reported back (or
    /// in a test, which never starts one).
    update_check: Option<UpdateCheck>,
    /// A newer build was found and is waiting on an answer — see
    /// [`Self::draw_update_prompt`].
    update_available: Option<(String, std::path::PathBuf)>,
    /// Where to update from, once it is safe to quit — set by **Update now**
    /// and carried out by [`Self::exit_program`]. An update is a quit that relaunches
    /// a newer build, not a separate "is anything unsaved" check of its own.
    pending_update: Option<std::path::PathBuf>,
}

/// Where a newer build is published — a Dropbox-synced folder, the same one
/// on every PC this runs on, so "push an update" is just copying files there
/// (see `build.ps1 -Publish`). A fixed path, not a setting: nothing about it
/// varies per install. If it is missing on some machine, [`read_update_manifest`]
/// just finds nothing, silently — never an error.
const UPDATE_FOLDER: &str = r"D:\Dropbox\YASEEN\pagify desktop";

/// Whether `candidate` is a genuinely newer version than `current` —
/// dot-separated numeric segments compared as a tuple, so `0.2.0` beats
/// `0.1.9` correctly rather than as strings. Not `semver`: this is one
/// team's own build counter, not a public package needing pre-release or
/// build-metadata rules.
fn newer_version(current: &str, candidate: &str) -> bool {
    fn parts(v: &str) -> Vec<u32> {
        v.trim().split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }
    parts(candidate) > parts(current)
}

/// The version published in `dir/version.txt`, trimmed — or `None` for any
/// reason at all (the folder is missing, unreadable, a Dropbox placeholder
/// not yet synced, empty). Every one of those means the same thing to a
/// caller: nothing to report.
fn read_update_manifest(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("version.txt")).ok()?;
    // A stray leading byte-order mark — Notepad, and older PowerShell's own
    // `-Encoding utf8`, both write one by default — is not whitespace, so
    // `.trim()` alone leaves it sitting in front of the first digit.
    let version = text.trim().trim_start_matches('\u{FEFF}').trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// What identifies a font program among the faces the app has been handed: a
/// hash of its length and first 4 KiB. The editor's own face
/// (`PagifyApp::editor_face`) and the name a face is registered under for
/// typing (`registered_face_of`) are both this, so two runs in one font
/// share both.
fn font_key(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut hasher);
    bytes[..bytes.len().min(4096)].hash(&mut hasher);
    hasher.finish()
}

/// **How much a page may hold and still have its paragraphs read in one pass at
/// a click** — the more of either, the longer the window stands still at the
/// first click on it. Past either number the click opens the word alone, found
/// from the page's text objects, read once (see `PagifyApp::page_weight` and
/// `PagifyApp::light_page`).
///
/// The page is read by opening it five times over (its text, its fonts' styles,
/// their names, its shapes, and the count that is asked first) and detecting
/// paragraphs in what was read; opening a page costs about 1.4 microseconds an
/// object, reading text about five milliseconds per thousand words, and the
/// detector grows faster than the number of words. Measured (see the
/// `heavy_page_costs` measurement, release build): the datasheet's pages — 900 to
/// 1 500 text objects, 1 100 to 2 900 objects — take 50 to 100 ms at the first
/// click; a drawing of 30 000 objects takes a quarter of a second, one of 64 000
/// about half a second, one of 877 000 about twenty seconds. Before the page
/// text read was made linear 5 000 words took one and a half seconds and 39 000
/// words five minutes; they are 30 and 230 milliseconds now.
///
/// **The text limit keeps a first click near a tenth of a second** (the detector
/// takes 30 ms at 6 000 words and 140 at 10 000). **The object limit is the one
/// the pick-path review proposed** (60 000), where a first click is half a second
/// and the freezes it was written for — 0.9 to 3 s on the plans in the review's
/// corpus, 10 to 45 s on a plan of 877 000 shapes — begin: below it the paragraphs
/// of a drawing are worth the wait of a quarter of a second once per page. (A
/// tighter limit of 12 000 objects was tried: it turned every click on 14 real CAD
/// pages into a single run and lost 140 of the 5 010 clicks the real-document set
/// scores exactly.)
const HEAVY_PAGE_TEXT_OBJECTS: usize = 6_000;
const HEAVY_PAGE_OBJECTS: usize = 60_000;

/// Whether a page is too big to read in one pass for its paragraphs — see
/// [`HEAVY_PAGE_TEXT_OBJECTS`].
fn page_is_heavy(weight: &pdf_core::document::PageScale) -> bool {
    weight.text_objects > HEAVY_PAGE_TEXT_OBJECTS || weight.page_objects > HEAVY_PAGE_OBJECTS
}

/// What an editor whose page changed under it is told — see
/// [`PagifyApp::editor_is_stale`]. Nothing was written, and the way on is to
/// click the words again.
const STALE_EDITOR_MESSAGE: &str =
    "the page changed since this box was opened: nothing was changed - click the text again.";

/// What the editor needs of one font program, worked out once: the program
/// itself, its metrics, the key it is known by and the characters it has ink
/// for. See [`FaceCache`].
#[derive(Clone)]
struct CachedFace {
    /// `None` where the run's font has no program to give (not embedded).
    program: Option<std::rc::Rc<Vec<u8>>>,
    /// `None` where the program cannot be read as a font.
    metrics: Option<pdf_core::pdf::embed::Metrics>,
    key: u64,
    coverage: Option<std::sync::Arc<pdf_core::pdf::embed::OutlineCoverage>>,
}

/// What a [`FaceCache`] entry is for: `(document, page, render epoch, undo
/// generation, font resource)`. The first four are the stamp the page's text is
/// read under (see `PagifyApp::page_blocks`) — a face is the page's, and a page
/// that has changed may have different fonts under the same numbers — and the
/// last is the page's own identity for the font (`RunStyle::font`).
type FaceKey = (u64, usize, u64, u64, u32);

/// **The font programs the editor has been handed, most recent first, and no
/// more than [`FaceCache::CAPACITY`] of them.**
///
/// Asking the engine for a run's font program opens the page again (4.5 to 6 ms
/// on a datasheet page, 80 ms on a drawing of tens of thousands of shapes), and
/// parsing it for its metrics and its ink coverage is more again — and every
/// click on a paragraph asked, a page of paragraphs in the same few fonts. Under
/// the stamp it was read under it is the same answer, so it is kept; **dropped
/// with the stamp**: an entry for any other state of the document is of no use
/// and is forgotten the next time the cache is looked at.
#[derive(Default)]
struct FaceCache {
    entries: Vec<(FaceKey, CachedFace)>,
}

impl FaceCache {
    const CAPACITY: usize = 16;

    /// The face for `key`, which becomes the most recent. **Entries made under
    /// another state of the same document go** (its epoch or its generation has
    /// moved: they describe a page that is not there any more); those of other
    /// pages in the same state, and of other documents (another tab), are still
    /// right and stay, within the capacity.
    fn get(&mut self, key: &FaceKey) -> Option<CachedFace> {
        self.entries.retain(|(held, _)| held.0 != key.0 || (held.2, held.3) == (key.2, key.3));
        let at = self.entries.iter().position(|(held, _)| held == key)?;
        let entry = self.entries.remove(at);
        let face = entry.1.clone();
        self.entries.insert(0, entry);
        Some(face)
    }

    fn put(&mut self, key: FaceKey, face: CachedFace) {
        self.entries.retain(|(held, _)| *held != key);
        self.entries.insert(0, (key, face));
        self.entries.truncate(Self::CAPACITY);
    }
}

/// What [`PagifyApp::paragraph_edit_commands`] decided.
struct ParagraphCommands {
    /// What to execute, in order — see [`paragraph_lines::commands_for`]: the
    /// paragraph's new colour, if one was asked for, and then the replace of
    /// its lines. Empty when nothing on the page has to change.
    commands: Vec<pdf_core::command::Command>,
    /// Frozen lines whose typed text differs from what they said — see
    /// [`paragraph_lines::ParagraphPlan::frozen_changed`].
    frozen_changed: usize,
    /// A new position was asked for. A paragraph is several lines and has no
    /// single place to move to, so it was not applied; the apply says so.
    position_ignored: bool,
    /// Typed lines with no original line left to hold them, to be written
    /// below the paragraph **after** `commands` have run — see
    /// [`SurplusLines`].
    surplus: Option<SurplusLines>,
}

/// New lines typed past what a paragraph had, written below it in its own look.
///
/// **Geometric, so written after the replace**: they are placed by a position,
/// not by an object number, and the replace renumbers the page. Written after
/// it also means a refused replace writes none of them, where written first
/// they were left on the page with the edit they belonged to refused.
struct SurplusLines {
    /// Where the first one goes: the left edge, and the line they go below.
    base_x: f32,
    below: f32,
    /// The distance between lines.
    gap: f32,
    /// The font they are written in, when the paragraph's own can be named.
    face: Option<String>,
    /// The typed lines, an empty one standing for a blank line.
    lines: Vec<String>,
}

impl SurplusLines {
    /// How many are real words (not blank lines).
    fn written(&self) -> usize {
        self.lines.iter().filter(|line| !line.is_empty()).count()
    }
}

/// One visual line of a paragraph as an editor is built from it — see
/// [`PagifyApp::build_editor_from_lines`].
struct ParagraphLine {
    /// The text objects that draw it, left to right. **Empty is legal when
    /// `frozen`**: a whole line drawn as shapes has none.
    objects: Vec<usize>,
    /// The line's own box — every member of it, shapes included.
    rect: pdf_core::document::Rect,
    /// Whether applying an edit must leave the line alone — see
    /// [`EditingRun::frozen`].
    frozen: bool,
}

/// The smallest rectangle holding both.
fn union_rect(a: pdf_core::document::Rect, b: &pdf_core::document::Rect) -> pdf_core::document::Rect {
    pdf_core::document::Rect {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

/// What a person is told when the engine **refuses** a text edit on purpose.
///
/// **Reported from use: "that text is drawn in a way this cannot edit is not
/// implemented yet".** `PdfError::Unsupported` prints "<reason> is not
/// implemented yet" — right for a feature nobody has built, wrong for the
/// twenty-odd places on the text-edit path where the engine looked at a page,
/// saw that it could not change the words *safely*, and said no. A protective
/// refusal read like a crash, and said nothing about what could be done.
///
/// **Done here, not in `pdf_core`.** `Unsupported` is also control flow in the
/// engine — `move_object` falls through to its guarded path on it, and a batch
/// that needs its one-at-a-time retry asks for it by matching the reason — so
/// its type and its `Display` are left exactly as they are, and every other
/// caller keeps the wording it has. The engine's own reason stays inside the
/// sentence; the session log keeps the raw error as well.
///
/// A reason with no entry here — a genuine gap such as "listing what a page
/// draws", or a refusal nobody has written a line for yet — comes back exactly
/// as the engine wrote it, never guessed at. So does every error that is not
/// `Unsupported`.
fn explain(e: &PdfError) -> String {
    /// The lookup refusals: the words could not be matched to one of the page's
    /// own text instructions.
    const LOOKUP: &str = "the words cannot be matched to the page's own text code with certainty, \
        so this refuses rather than risk changing the wrong ones. A different word or line may \
        still work; if none does, the program that made the PDF can change it.";
    /// The font-structure refusals: the font's codes cannot be read as letters.
    const FONT_MAP: &str = "the page does not say, in a way this can read, which letter each code of \
        that font stands for, so changing these words would be a guess and this refuses instead. \
        A different word or line may still work.";
    /// The page-rewrite guard: PDFium's own rewrite would scramble other text.
    const REWRITE: &str = "rewriting the page around it could scramble other text, so this refuses.";

    let PdfError::Unsupported(reason) = e else { return e.to_string() };
    let tail: String = match *reason {
        "that is not a text run"
        | "that text is drawn in a way this cannot edit"
        | "that run's codes cannot be lined up with its text"
        | "that run's characters belong to none of its codes" => LOOKUP.into(),
        "that text selects no font"
        | "that page declares no fonts"
        | "that font's codes cannot be counted"
        | "that font carries no character map this can read" => FONT_MAP.into(),
        r if r.starts_with("this run's colour cannot be changed without rewriting") => {
            format!("{REWRITE} Retyping the words is not affected.")
        }
        r if r.starts_with("this run cannot be changed without rewriting") => {
            format!("{REWRITE} A different word or line may still work.")
        }
        "this run's font cannot write those characters" => "it holds only the letters the document \
            already uses. Apply the new words on their own, without a new Position or Colour, or \
            use only letters that are already in them."
            .into(),
        // Not clauses: the engine's words only read as a sentence with the
        // suffix on, so they are replaced rather than extended.
        r if r.starts_with("changing how these words look rewrites the rest of the page") => {
            return "changing a run's position, or its colour along with its words, rewrites the \
                rest of this page, which would disturb other text, so nothing was changed. \
                Retyping the words on their own does not hit this limit."
                .into();
        }
        "editing two runs that one operator draws" => {
            return "two of the pieces being changed are drawn by one instruction in the page's \
                code and cannot be told apart, so this refuses rather than change both. A \
                different word or line may still work."
                .into();
        }
        _ => return e.to_string(),
    };
    format!("{reason} — {tail}")
}

/// The batch script that carries out an update: wait for process `pid` to
/// exit, put the new `Pagify.exe` and `pdfium.dll` from `source` where the old
/// ones are in `install_dir`, check the exe really is the new one, relaunch.
///
/// **Reported from use: "when I click Update now it brings the same update
/// window back".** Another Pagify window was open from the same folder. Windows
/// does not let a running program's file be overwritten, the script waited only
/// for *this* process, its `copy` failed with nothing to say so, and the old
/// build relaunched and offered the same update again — every time.
///
/// So a file that cannot be overwritten is **renamed** out of the way first,
/// which Windows does allow for a running program (the other window carries on
/// from the renamed copy, and the leftover is cleared by a later update); the
/// result is checked; a copy that cannot be finished puts the old file back
/// rather than leave no exe at all; and a failure is said aloud (`notify`)
/// and logged to `log`, not discovered as the same prompt again.
fn update_script(
    pid: u32,
    source: &std::path::Path,
    install_dir: &std::path::Path,
    log: &std::path::Path,
    notify: bool,
) -> String {
    let warn = if notify {
        "powershell -NoProfile -Command \"Add-Type -AssemblyName System.Windows.Forms; \
         [void][System.Windows.Forms.MessageBox]::Show('Pagify could not install the update: \
         its files are in use or its folder cannot be written. Close every Pagify window and \
         choose Update now again.', 'Pagify update')\""
    } else {
        "rem"
    };
    format!(
        r#"@echo off
set "sys=%SystemRoot%\System32"
goto main
:put
copy /Y %1 %2 >NUL 2>&1 && exit /b 0
set "old=%~nx2.old-%RANDOM%"
ren %2 "%old%" >NUL 2>&1 || exit /b 1
copy /Y %1 %2 >NUL 2>&1 && exit /b 0
ren "%~dp2%old%" "%~nx2" >NUL 2>&1
exit /b 1
:main
:wait
"%sys%\tasklist.exe" /FI "PID eq {pid}" | "%sys%\find.exe" "{pid}" >NUL
if not errorlevel 1 (
    "%sys%\PING.EXE" -n 2 127.0.0.1 >NUL
    goto wait
)
del /Q "{dir}\*.old-*" >NUL 2>&1
call :put "{source_exe}" "{dest_exe}"
call :put "{source_dll}" "{dest_dll}"
"%sys%\fc.exe" /b "{source_exe}" "{dest_exe}" >NUL 2>&1
if errorlevel 1 goto failed
echo %date% %time% updated>>"{log}"
goto relaunch
:failed
echo %date% %time% FAILED: the new Pagify.exe is not in place>>"{log}"
{warn}
:relaunch
start "" "{dest_exe}"
(goto) 2>NUL & del "%~f0"
"#,
        dir = install_dir.display(),
        source_exe = source.join("Pagify.exe").display(),
        dest_exe = install_dir.join("Pagify.exe").display(),
        source_dll = source.join("pdfium.dll").display(),
        dest_dll = install_dir.join("pdfium.dll").display(),
        log = log.display(),
    )
}

/// Wait for this process to exit, then put a newer build over it and relaunch
/// — see [`update_script`]. Written out as a batch script and launched
/// detached, because `pdfium.dll` stays locked open for as long as this
/// process has it loaded, and nothing here can replace it out from under
/// itself. If writing or launching the script fails, the update does not
/// happen — the exit this was about to do already outweighs a dialog nobody
/// can act on this late; what the script itself cannot finish it says.
fn spawn_update_script(source: &std::path::Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(install_dir) = exe.parent() else { return };
    let pid = std::process::id();
    let log = std::env::temp_dir().join("pagify-update.log");
    let script = update_script(pid, source, install_dir, &log, true);
    let path = std::env::temp_dir().join(format!("pagify-update-{pid}.bat"));
    if std::fs::write(&path, script).is_err() {
        return;
    }
    let _ = std::process::Command::new("cmd").arg("/C").arg(&path).spawn();
}

/// What a redaction has to add about forms the file draws elsewhere: the
/// words came off this page's own copy, and stay where the other pages draw
/// the same form. Empty when there is nothing of the kind.
fn shared_form_notes(report: &pdf_core::document::RedactionReport) -> String {
    let notes: Vec<String> = report
        .uncleared
        .iter()
        .filter(|u| matches!(u, pdf_core::document::Uncleared::SharedForm { .. }))
        .map(|u| u.describe())
        .collect();
    if notes.is_empty() {
        String::new()
    } else {
        format!(" Note: {}.", notes.join("; "))
    }
}

/// Who a signature is by, for a readout.
///
/// **The certificate's subject when the signature verified; otherwise the
/// file's own label, marked as such.** The `/Name` in a signature dictionary
/// is plain text written by whoever wrote the file, and printing it bare
/// beside "unchanged" let it read as a finding. Found by audit.
fn signer_label(signature: &pdf_core::pdf::validate::Signature) -> String {
    match (&signature.signer, signature.name.trim()) {
        (Some(subject), _) => format!(" by {subject}"),
        (None, "") => String::new(),
        (None, label) => format!(" by {label} (as the file labels it — not verified)"),
    }
}

/// One signature, in one line: what it is called here, who signed, the
/// verdict about the bytes — and, when a signer was verified at all, what the
/// pinned roots say about the signer, kept apart from the verdict with a
/// semicolon because they are different questions. The tick comes only when
/// both answers are the good one: unchanged, and issued by a root Pagify
/// trusts. A signature Pagify does not verify — another scheme, another
/// application — gets the verdict alone, which says "not verified" and never
/// "changed".
fn signature_line(what: &str, signature: &pdf_core::pdf::validate::Signature) -> String {
    let tick = if signature.is_good() { "✓ " } else { "" };
    let trust = match signature.trust {
        Some(trust) => format!("; {}", trust.describe()),
        None => String::new(),
    };
    format!(
        "{tick}{what}{}: {}{trust}",
        signer_label(signature),
        signature.verdict.describe()
    )
}

/// Whether a signature's line is news or a warning: anything but unchanged
/// is a warning, and so is a signer since revoked, however unchanged.
fn signature_is_a_warning(signature: &pdf_core::pdf::validate::Signature) -> bool {
    signature.verdict != pdf_core::pdf::validate::Verdict::Unaltered
        || signature.trust == Some(pdf_core::pdf::trust::Trust::Revoked)
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

/// Open `url` with whatever this platform normally opens one with — a web
/// link is only worth adding if clicking it does the one thing a link is
/// for.
///
/// **Passed as its own argument, never through a shell.** A URL is not
/// something this program wrote — a link clicked here can come from any PDF
/// somebody opened — and a query string's own `&` is a second command to
/// `cmd /C`, not a character in an address. Every branch below hands the OS
/// launcher the URL as a single `arg`, which `std::process::Command` passes
/// straight through the platform's own process-creation call rather than
/// interpreting it, so nothing in it is ever read as a command separator.
fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // `rundll32`'s own command line syntax, not this program's choice:
        // `<dll>,<entry point>` first, then whatever that entry point takes
        // — here, the one URL to open. Older but steadier than shelling out
        // to `cmd /C start`, which treats a bare URL as the window title
        // unless it is quoted just so.
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}


/// A run of text open for editing, in place.
#[derive(Clone)]
struct EditingRun {
    page: usize,
    /// The first object of the first line that has one — `lines[0].0[0]`
    /// unless that line is drawn entirely as shapes and has no object. Kept
    /// alongside `lines` rather than derived from it at every use, since
    /// most of what reads this field only ever cared about the one-line
    /// case, where it is the only object there is.
    object: usize,
    /// What it said when it was picked, so undo and "unchanged" both have
    /// something true to compare against. For a paragraph, every line's own
    /// text joined with `\n`, in the same top-to-bottom order as `lines`.
    original: String,
    /// The union of every line's own rectangle — the box the editor is
    /// drawn over. See [`Self::lines`] for each line's own rectangle on its
    /// own, which is what applying an edit needs to put each line back
    /// where it was.
    rect: pdf_core::document::Rect,
    /// Every line this edit covers, top to bottom, with the object(s) that
    /// draw it and its own combined rectangle. One entry for an ordinary
    /// run; more than one only when a whole paragraph was picked in Edit
    /// Text — see [`PagifyApp::pick_paragraph`]. `apply_one_edit` splits
    /// the typed text back across these on the way out, the same order it
    /// was joined coming in.
    ///
    /// **Usually one object, sometimes more.** A producer routinely splits
    /// one visual line across several text-showing operations — see
    /// `pagify_shell::blocks`' own doc — and this line's text is every one of
    /// them, left to right, concatenated. Retyping such a line writes the new
    /// text to the first object and removes the rest from the page; see
    /// `paragraph_lines::line_edits`.
    ///
    /// For a paragraph picked by a click these are the detector's own lines,
    /// one to one, never regrouped here: [`Self::buffer`] has exactly one
    /// `\n`-separated entry for each.
    lines: Vec<(Vec<usize>, pdf_core::document::Rect)>,
    /// One flag per entry of [`Self::lines`], same length: whether that line
    /// is **frozen**.
    ///
    /// A frozen line carries words the page draws as vector paths — an
    /// outlined word, a ligature, a whole line of art — next to or instead of
    /// its text objects. Rewriting its text objects would put the whole line's
    /// words into the first of them while the paths went on being drawn (the
    /// words twice over), and hiding it would leave the paths showing. So
    /// applying an edit leaves a frozen line exactly as the page has it,
    /// whatever was typed over it, and says so — see
    /// [`PagifyApp::paragraph_edit_commands`].
    ///
    /// **A frozen line may have no text object at all** (`lines[i].0` empty):
    /// a whole line drawn as shapes. Its place in [`Self::buffer`] is an empty
    /// line, so the buffer still has exactly one `\n`-separated entry per line.
    /// Always `false` for a run, or a paragraph, picked from text alone.
    frozen: Vec<bool>,
    /// For each of [`Self::lines`], the objects that **duplicate** one of its
    /// pieces — the faux bold of a heading, every letter drawn twice a hair
    /// apart — and have no words of their own to show in the buffer. Empty for a
    /// paragraph that has none, otherwise one list per line.
    ///
    /// They go with their line: retyping or deleting a line takes its twins off
    /// the page too (see `paragraph_lines::line_edits`), or they would go on
    /// drawing the old words over the new; a frozen or unchanged line's twins are
    /// never touched.
    twins: Vec<Vec<usize>>,
    /// The document's command-history generation —
    /// `pagify_shell::session::Session::undo_generation` — at the moment this
    /// was picked.
    ///
    /// This editor holds page-object numbers, and anything that moves the
    /// generation between the pick and the apply — an edit, an undo, a redo —
    /// can renumber the page's objects under it, so that applying would write
    /// the typed words onto other objects. `apply_editing_page` refuses to
    /// write when it has moved. See also [`Self::render_epoch`], the other half
    /// of the stamp.
    doc_generation: u64,
    /// The document's [`Doc::render_epoch`] at the moment this was picked.
    ///
    /// **The other half of the stamp, for the page changes that are not in the
    /// undo history.** Restacking an object (the Layers panel), splitting a run
    /// into its letters, a lock, a stamp, a whiteout, an import: each renumbers
    /// the page's objects and moves *no* generation, so the editor — which names
    /// objects by number — would write onto whatever now has them. Every one of
    /// them says the page changed through [`Doc::rendered_is_stale`], which moves
    /// this; so an editor whose epoch is no longer the document's is stale
    /// ([`PagifyApp::editor_is_stale`]): refused at apply, and closed on the
    /// next frame. (Reported by the pick-path review, with a reproduction: a
    /// paragraph retyped after a Layers-panel restack overwrote a word outside
    /// it.) Turning the *view* also moves it — a false alarm, and a harmless one:
    /// the text is clicked again.
    render_epoch: u64,
    /// What the engine said when it refused this edit, with the edit it refused
    /// — kept so that **the typing is not thrown away with the refusal** and so
    /// that a second click away from an edit that has not changed since lets it
    /// go instead of asking again. `None` until the engine has refused it.
    refusal: Option<EditRefusal>,
    /// What is being typed.
    buffer: String,
    /// The appearance, as it will be applied. Seeded from the run so that
    /// leaving the controls alone changes nothing.
    style: pdf_core::document::TextStyle,
    /// The appearance as it was, so "did anything change" is answerable
    /// without asking the document again.
    was: pdf_core::document::TextStyle,
    /// The name of the font this run is actually drawn in right now, read
    /// once when it was picked — for the font field's label only.
    ///
    /// **Not the same thing as `style.face`.** That field means "write the
    /// run in exactly this named font instead of the automatic choice", and
    /// setting it from the run's own current font would have silently
    /// changed Apply's behaviour: picking a font by an exact name refuses
    /// outright rather than substituting if that font cannot spell the
    /// text, where leaving `style.face` at `None` (its correct default,
    /// meaning "automatic") never would have. This field exists so the label
    /// can stop lying and say "Montserrat-Bold" instead of "(automatic)"
    /// without smuggling an unasked-for font change into what Apply does.
    /// `None` for drawn (outlined) words, which have no font to name.
    current_face: Option<String>,
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
    /// The text object whose face, size and colour the box is set in: the
    /// paragraph's dominant look (see [`majority_look`]) for a paragraph, the
    /// run itself for a single run, `usize::MAX` for drawn words. What
    /// `want_document_face` was asked for; kept so a test, or the census, can
    /// say which look won.
    look_object: usize,
    /// Whether this editor has been given keyboard focus yet — see
    /// `draw_run_editor`'s own use of it. Requested once, the first frame
    /// it is drawn, not every frame, or typing could never hand focus back
    /// to whatever else gets clicked afterwards.
    focused: bool,
    /// How the reader has dragged the box open round this run — see [`RunBox`].
    box_resize: RunBox,
}

/// Runs a person declared one paragraph by hand — see [`DocTab::joined_groups`].
///
/// **It names the runs by object number, and object numbers do not last**: a
/// deleted object, a restack, an undo that puts a piece back, a page imported
/// ahead of it — each renumbers the page, and the group would then name other
/// words. So it also carries a **fingerprint** of what it named — how many runs,
/// and a hash of each one's number, words and box — checked against the page as
/// it is **before every use**; a group whose fingerprint no longer matches is
/// forgotten, not guessed at (see [`PagifyApp::forget_stale_groups`]). Reported by
/// the pick-path review with a reproduction: after an object was deleted ahead of a
/// joined pair, a click on an unrelated word opened a joined editor with two
/// unrelated words glued together, and an Apply would have overwritten both.
#[derive(Debug, Clone, PartialEq)]
struct JoinedGroup {
    page: usize,
    /// The runs, top to bottom.
    objects: Vec<usize>,
    fingerprint: u64,
}

/// A fingerprint of `objects` as `pb` reads them — their count, and for each its
/// number, words and box — or `None` when one of them is not on the page.
fn group_fingerprint(pb: &PageBlocks, objects: &[usize]) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    objects.len().hash(&mut hasher);
    for object in objects {
        let run = pb.runs.get(object)?;
        object.hash(&mut hasher);
        run.text.hash(&mut hasher);
        for edge in [run.rect.left, run.rect.top, run.rect.right, run.rect.bottom] {
            edge.to_bits().hash(&mut hasher);
        }
    }
    Some(hasher.finish())
}

/// An edit the engine refused, as the editor remembers it: why, and exactly
/// what was refused — see [`EditingRun::refusal`].
#[derive(Clone)]
struct EditRefusal {
    /// What was said to the person.
    reason: String,
    /// The words and the look that were refused. An edit that is the same again
    /// would be refused for the same reason, so it is not asked a second time.
    buffer: String,
    style: pdf_core::document::TextStyle,
}

impl EditRefusal {
    fn matches(&self, edit: &EditingRun) -> bool {
        self.buffer == edit.buffer && self.style == edit.style
    }
}

/// How a line sits within [`NewTextBox::rect`]'s width.
///
/// Only three, not four: a justified line needs its inter-word gaps
/// stretched, which egui's own wrap has no concept of and this app has no
/// existing machinery for — see [`PagifyApp::apply_new_text_box`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextAlign {
    Left,
    Center,
    Right,
}

/// Text being composed for a brand new spot on the page — the click-and-drag
/// box `addtext` (bare) arms, as opposed to [`EditingRun`], which is always
/// an *existing* run already in the page's content.
///
/// **Reported from use: adding text meant typing the words into the command
/// box before knowing where they would land, "totally confusing and
/// unintuitive."** This is the box instead: drag out the area first, type
/// into it, and only turn it into real page content when it is confirmed —
/// see [`PagifyApp::begin_text_box`] and [`PagifyApp::apply_new_text_box`].
struct NewTextBox {
    page: usize,
    /// The dragged box, in page/app space — the width wraps the typing, and
    /// [`Self::align`] decides where each wrapped line sits inside it.
    rect: pdf_core::document::Rect,
    buffer: String,
    size: f32,
    color: pdf_core::document::Color,
    /// An explicit pick from the same font picker a run's own edit uses —
    /// see [`PagifyApp::draw_font_picker`]. `None` writes in the base
    /// Helvetica `write_text_at` already uses for a single click.
    face: Option<String>,
    align: TextAlign,
    /// Focus is asked for once — see [`EditingRun::focused`].
    focused: bool,
}

/// What the reader has done to the box open round a run, by dragging the
/// handles on its left and right edges — see `PagifyApp::draw_run_editor`.
/// Everything is in page points, so it holds still as the page is zoomed.
///
/// **A run's box is a width to wrap the words to.** Dragged narrower the words
/// fold onto more lines, wider they come back onto fewer; either way what lands
/// on the page is what the box showed (see `apply_one_edit`'s extra lines).
/// ponytail: a resize joins the box's lines into one before it wraps again, so
/// a line break the reader typed by hand is lost to it; keep hand breaks apart
/// from wrapped ones if that is ever reported.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct RunBox {
    /// The width dragged to; `None` is the width the run opened at.
    width_pt: Option<f32>,
    /// How far the left edge has been dragged from where the run starts. The
    /// run moves with it: applying takes it into [`TextStyle::at`].
    left_shift_pt: f32,
    /// The width changed since the buffer was last wrapped, so the next frame
    /// joins its lines and wraps them afresh.
    reflow: bool,
}

/// The narrowest a box can be dragged to, in page points.
const MIN_RUN_BOX_PT: f32 = 12.0;

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

/// A newer-build check running on its own thread — see
/// [`PagifyApp::collect_update_check`]. Reading [`UPDATE_FOLDER`] is a
/// filesystem call, on a folder that may be a Dropbox mount not currently
/// syncing or simply missing; it must never hold the window up at launch,
/// so it gets the same "spawn a thread, poll a channel" treatment `Reading`
/// above already uses for OCR.
struct UpdateCheck {
    /// `Some((newer version, the folder it lives in))` once a genuinely
    /// newer build is found; `None` for "nothing newer" or "could not even
    /// look" — the two are not told apart, since neither is ever worth
    /// telling the user about.
    done: std::sync::mpsc::Receiver<Option<(String, std::path::PathBuf)>>,
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
    /// never selected — see `Session::lock_shapes`.
    ///
    /// **Shapes rather than one rectangle**, because a selection over two lines
    /// is not a rectangle: the smallest one holding it also holds the head of
    /// the first line and the tail of the last. Collapsing them here is what
    /// made a 39-character selection take 68 characters and the word before it.
    /// A dragged rectangle is simply a list of one.
    Lock { page: usize, shapes: Vec<pdf_core::document::Rect>, require_complete: bool },
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
/// Something the object tool has picked up.
#[derive(Debug, Clone, PartialEq)]
struct Selected {
    page: usize,
    object: usize,
    rect: pdf_core::document::Rect,
    what: &'static str,
}

/// See [`PagifyApp::right_click_text_actions`]'s own doc for why this
/// exists as a cache rather than being computed where it is used.
#[derive(Debug, Clone, Copy, Default)]
struct RightClickTextActions {
    joinable: bool,
    split_object: Option<usize>,
}

/// A placed-but-unapplied picture signature, picked with no tool armed.
/// `index` is the annotation's PDFium index — [`pdf_core::document::
/// ImageSignatureMark::index`] — not a page-content object number, so it is
/// never confused with [`Selected::object`] even though both are `usize`.
#[derive(Debug, Clone, PartialEq)]
struct SignatureSelected {
    page: usize,
    index: usize,
    rect: pdf_core::document::Rect,
    /// How far clockwise it is turned about its own centre, in degrees, as
    /// of the moment it was (re)selected — the base a rotate-drag's angle
    /// is added onto, not recomputed from scratch on every frame.
    rotation: f32,
}

/// A plain placed picture, picked with no tool armed — the same shape as
/// [`SignatureSelected`] and kept every bit as separate from it as
/// `PlacedImageMark` is from `ImageSignatureMark`: reusing one field for
/// both would mean a single click could only ever have selected one or the
/// other, and would blur exactly the line `apply_signatures` depends on.
#[derive(Debug, Clone, PartialEq)]
struct PlacedImageSelected {
    page: usize,
    index: usize,
    rect: pdf_core::document::Rect,
    rotation: f32,
}

/// What `copy` on a selected drawn shape or placed picture carries to
/// `paste` — see [`PagifyApp::object_clipboard`].
///
/// A drawn shape is kernel geometry (`Layer::add_object`); a placed picture
/// is an annotation (`Command::AddAnnotation`). They paste through
/// completely different engine calls, so the clipboard keeps them as two
/// variants rather than forcing one shape on both.
#[derive(Clone)]
enum ObjectClipboard {
    /// Each shape with whether it was filled — carried separately rather
    /// than by including its paired `Geom::Hatch` in the list, because a
    /// Hatch names its boundary by *handle*, and a copy gets a fresh one
    /// (see `tools::copy_selection`'s doc). A cloned Hatch would still point
    /// at the original's handle, filling the original a second time instead
    /// of the paste.
    Shapes(Vec<(cad_kernel::DObject, bool)>),
    Image { rgba: Vec<u8>, width: u32, height: u32, rect: pdf_core::document::Rect },
    /// Words — a run or a paragraph, one entry per line — with what they were
    /// drawn in. Pasted as new text, so it stays text.
    Text { lines: Vec<String>, size: f32, color: pdf_core::document::Color, face: Option<String> },
}

/// What the system clipboard is given after a copy that is not text, so the
/// ⌘V that follows has something to paste — see
/// [`PagifyApp::clipboard_mirror_wanted`]. Also how a paste tells "my own copy
/// is the newest" from "text was copied since, somewhere else".
const COPIED_IN_PAGIFY: &str = "(copied in Pagify)";

/// A paste waiting for a click: the copy follows the pointer at half
/// strength and is put down only where the reader clicks, so it never lands
/// somewhere they did not choose. See [`PagifyApp::paste_ghost`].
struct PasteGhost {
    content: ObjectClipboard,
    /// A picture's pixels as a texture, made the first frame it is drawn.
    texture: Option<egui::TextureHandle>,
}

/// What Organize's own `copy` carries to `paste` — see
/// [`PagifyApp::page_clipboard`]. A path rather than bytes in memory: the
/// same shape [`pagify_shell::Session::extract_to`]/`import_from` already
/// use for page-to-page copying, so pasting is just handing that same path
/// to `import_from` on whichever tab is the paste target.
struct PageClipboard {
    temp_file: std::path::PathBuf,
    page_count: usize,
}

// No `Drop` that deletes the file: the copy belongs to every Pagify window,
// and closing the one that made it must not take the pages with it. The next
// copy, from any window, clears the earlier files (`publish_page_clipboard`).

/// One of the eight places on a selection's outline that resizes it.
///
/// Named by which sides move: dragging the right edge changes the right side
/// only; a corner changes two. The anchor — the point that stays still — is
/// the opposite side or corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// A ninth handle, for a signature's selection alone — see
    /// [`PagifyApp::signature_handle_at`]. Deliberately left out of
    /// [`Handle::ALL`], `at`, `anchor` and `scale`: those four are the
    /// object tool's own resize geometry, proven and in wide use, and this
    /// handle does not resize anything — turning is a different transform,
    /// computed from where the drag is relative to the rect's centre
    /// (`PagifyApp::angle_from_drag`), not from a fixed corner or edge.
    Rotate,
}

impl Handle {
    const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
    ];

    /// Where this handle sits on a rectangle. Never called with `Rotate` —
    /// its position depends on the view's scale (a fixed screen-space
    /// offset, not a page-space one), which this method is not given; see
    /// [`PagifyApp::rotate_handle_screen_pos`].
    fn at(&self, r: &pdf_core::document::Rect) -> (f32, f32) {
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        match self {
            Handle::Left => (r.left, cy),
            Handle::Right => (r.right, cy),
            Handle::Top => (cx, r.top),
            Handle::Bottom => (cx, r.bottom),
            Handle::TopLeft => (r.left, r.top),
            Handle::TopRight => (r.right, r.top),
            Handle::BottomLeft => (r.left, r.bottom),
            Handle::BottomRight => (r.right, r.bottom),
            Handle::Rotate => unreachable!("the rotate handle's position is screen-space, not page-space"),
        }
    }

    /// The point that does not move when this handle is dragged. Never
    /// called with `Rotate` — turning has no anchor point, it has a centre
    /// (see [`PagifyApp::angle_from_drag`]).
    fn anchor(&self, r: &pdf_core::document::Rect) -> (f32, f32) {
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        match self {
            Handle::Left => (r.right, cy),
            Handle::Right => (r.left, cy),
            Handle::Top => (cx, r.bottom),
            Handle::Bottom => (cx, r.top),
            Handle::TopLeft => (r.right, r.bottom),
            Handle::TopRight => (r.left, r.bottom),
            Handle::BottomLeft => (r.right, r.top),
            Handle::BottomRight => (r.left, r.top),
            Handle::Rotate => unreachable!("the rotate handle has no anchor corner"),
        }
    }

    /// The scale a drag of `by` page points means, for a rectangle this size.
    fn scale(&self, r: &pdf_core::document::Rect, by: (f32, f32)) -> (f32, f32) {
        let (w, h) = ((r.right - r.left).max(0.01), (r.bottom - r.top).max(0.01));
        let sx = match self {
            Handle::Left | Handle::TopLeft | Handle::BottomLeft => (w - by.0) / w,
            Handle::Right | Handle::TopRight | Handle::BottomRight => (w + by.0) / w,
            _ => 1.0,
        };
        let sy = match self {
            Handle::Top | Handle::TopLeft | Handle::TopRight => (h - by.1) / h,
            Handle::Bottom | Handle::BottomLeft | Handle::BottomRight => (h + by.1) / h,
            _ => 1.0,
        };
        (sx.max(0.05), sy.max(0.05))
    }

    fn cursor(&self) -> egui::CursorIcon {
        match self {
            Handle::Left | Handle::Right => egui::CursorIcon::ResizeHorizontal,
            Handle::Top | Handle::Bottom => egui::CursorIcon::ResizeVertical,
            Handle::TopLeft | Handle::BottomRight => egui::CursorIcon::ResizeNwSe,
            Handle::TopRight | Handle::BottomLeft => egui::CursorIcon::ResizeNeSw,
            // No dedicated rotate cursor exists in egui; crosshair reads as
            // "precise work happening here" without implying a direction
            // that would be wrong half the time.
            Handle::Rotate => egui::CursorIcon::Crosshair,
        }
    }
}

/// A drag on the selection, from press to release.
#[derive(Debug, Clone, PartialEq)]
struct Grab {
    /// A handle, or `None` for the body of the selection.
    handle: Option<Handle>,
    from: AppPoint,
    /// How far it has come, in page points.
    by: (f32, f32),
}

/// What a save did — or did not — do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveOutcome {
    Done,
    Failed,
    /// Nothing written yet: a question is up — see `PagifyApp::asking_to_secure`.
    Asking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Closing {
    /// Close the document, keep the program — `close`'s own, single-tab
    /// notion of closing: the tab stays, empty, showing the backstage view.
    Document,
    /// Close the program.
    Program,
    /// Close one tab outright, by index — the tab strip's × and Ctrl+W.
    /// Unlike `Document`, the tab itself goes away rather than going blank.
    Tab(usize),
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
    // `texture_for` separately clamps the result against the actual page's
    // own size (`whole_page_scale_ceiling`) before ever asking the GPU for a
    // texture, so this cap is not what stands between a large page and an
    // oversized allocation — it only bounded how sharp the page could look
    // on an ordinary-sized page before the zoom ceiling above it was raised
    // from 1600% to `PagifyApp::MAX_ZOOM` (6400%). Left at the old 32.0, a
    // HiDPI display (pixels_per_point above 1.0) would hit this before
    // reaching the new zoom ceiling at all, going soft well short of it.
    let clamped = device_scale.clamp(0.05, 96.0);
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
        A::Image { rect, .. } => Some(*rect),
        A::Link { rect, .. } => Some(*rect),
        A::Ink { .. } | A::Text { .. } | A::Fill { .. } => None,
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



/// Sized so a two-line name still leaves the glyph centred, and so a row of
/// them reads as a grid rather than as a ragged line.
/// Half the side of a resize handle, in screen pixels.
const HANDLE_PX: f32 = 4.0;

/// How far above a signature's own top edge the rotate handle floats, in
/// screen pixels — far enough that a finger or a cursor does not fight with
/// the resize handle right below it.
const ROTATE_HANDLE_OFFSET_PX: f32 = 26.0;
/// The rotate handle's own drawn (and hit-tested) radius, in screen pixels.
const ROTATE_HANDLE_PX: f32 = 8.0;

/// How wide a placed signature is, in page points — a signature on a form
/// is around two inches across; wider looks like a banner and narrower like
/// initials. Shared by [`PagifyApp::place_signature`] (what actually gets
/// placed) and [`PagifyApp::draw_signature_preview`] (what it looks like
/// before that), so the two cannot silently drift apart.
const SIGNATURE_WIDTH_PT: f32 = 144.0;








impl PagifyApp {

    /// Every frame's opening: theme sync, the face that was asked for last
    /// frame, every dialog and prompt, the close-request guard, and the
    /// collect-* polls. Split out of `ui` so that the frame function reads as
    /// the outline of a frame (design review Phase 1).
    fn draw_frame_preamble(&mut self, ctx: &egui::Context) {
        // Recomputed every frame rather than only when the View-tab toggle is
        // pressed, the same reasoning as `resolved_zoom` a few lines into
        // `draw_pages`: `theme::set_mode` has no `&egui::Context` of its own
        // to call `apply` with (`fn act` doesn't carry one), so the flag it
        // flips is picked up here, the one place every frame already passes
        // through, instead.
        theme::apply(ctx);

        // Escape while a tab is being dragged puts it back, and is not also
        // an Escape for everything else — see `hub`.
        self.cancel_tab_drag_on_escape(ctx);

        // Once a frame, so two separate actions landing between one undo
        // press and the next are still told apart in the order they actually
        // happened — see `track_undo_recency`'s own doc for why polling only
        // when `undo` is pressed loses exactly that ordering.
        self.track_undo_recency();

        if self.mark.is_none() {
            install_icons(ctx);
            self.mark = logo::texture(ctx);
        }

        // A face asked for while picking a run is installed here, at the top of
        // the frame after it was asked for — and marked usable only on the
        // frame after *that*, when egui has rebuilt its atlas. Drawing in a
        // family that does not exist yet panics; see `install_fonts`.
        if let Some(face) = self.pending_face.take() {
            install_fonts(ctx, Some(face));
        } else if self.editor_face.is_some() {
            self.editor_face_ready = true;
        }

        // A document that wants a password asks for it in a window, not in the
        // command box — see `draw_password_dialog`.
        self.draw_passcode_dialog(ctx);
        self.draw_signature_pad(ctx);
        self.draw_signature_list(ctx);
        self.draw_snippet_list(ctx);
        self.draw_find_replace(ctx);
        self.draw_spell_check(ctx);
        self.draw_bookmark_panel(ctx);
        self.draw_link_prompt(ctx);
        self.draw_extract_dialog(ctx);
        self.draw_article_box_prompt(ctx);

        // The close button is how people actually quit, and it bypasses every
        // verb. Without this the whole guard is decoration: `quit` refuses
        // politely while the red button throws the work away.
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(index) = self.tab_with_unsaved_work() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.active_tab = index;
                self.tab_mut().closing = Some(Closing::Program);
                // This window's button closes this window; whether that is
                // also the end of the program is the `Hub`'s to say.
                self.win.closing_leaves = hub::Leaving::Window;
            } else {
                self.leave(hub::Leaving::Window);
            }
        }
        self.ask_about_unsaved(ctx);
        self.ask_about_securing(ctx);
        self.ask_about_redaction(ctx);
        self.collect_reading(ctx);
        self.collect_spell_scan(ctx);
        self.collect_update_check(ctx);
        self.draw_update_prompt(ctx);
    }

    /// The keyboard half of a frame: the focus guard, Escape, copy/paste, the
    /// submit gate, document keys and the shortcuts. `command_id` and `frame`
    /// are passed in because widgets drawn later in the same frame need them
    /// too.
    fn handle_input(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame, command_id: egui::Id) {
        // The focus guard — §5.5. Read once, before any widget runs.
        let focus = Focus::capture(ctx);

        // **Space is a space.** It used to be a second Enter here — swallowed
        // from the box and the line run as it stood — on the strength of a
        // `Mode::TextContent` that nothing ever set. Reported from use as
        // "Extract only works as the raw command": `extract` and then a Space
        // ran `extract` before any page could follow it, so no line of more
        // than one word could be typed, and the ribbon's prefill-and-wait
        // buttons (Swap, Note, Calibrate…) could not be completed. Enter, the
        // Run button and the ribbon are the only ways to submit.

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
            // **Not a raw key press — confirmed against egui-winit 0.36.1's
            // own source.** On ⌘C/⌘V, `is_copy_command`/`is_paste_command`
            // match first and the translator pushes `Event::Copy`/
            // `Event::Paste` and returns *without* ever also emitting a
            // `Key::C`/`Key::V` press — so `i.key_pressed(egui::Key::C)` can
            // never be true for a real keyboard. It only looked like it
            // worked in this app's own tests because the test harness
            // constructs a synthetic `Event::Key` directly, skipping that
            // translation entirely. Reported from use: ⌘C/⌘V did nothing at
            // all, not even a "nothing selected" fallback — the dispatch
            // code below was simply never reached.
            copy: i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            // `Event::Paste` carries whatever text was already on the *system*
            // clipboard, and egui-winit only pushes it when that clipboard
            // already holds non-empty text — unrelated to whether this app
            // has anything of its own to paste. `copy_organize_selection`/
            // `copy_object_selection` mirror a short placeholder into the
            // system clipboard for exactly this reason, so the ⌘V that
            // follows a ⌘C always has *something* there to trigger this.
            paste: i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            find_next: i.key_pressed(egui::Key::Enter) && i.modifiers.command,
            open: i.modifiers.command && i.key_pressed(egui::Key::O),
            save: i.modifiers.command && i.key_pressed(egui::Key::S),
            undo: i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
            redo: i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z),
            search: i.modifiers.command && i.key_pressed(egui::Key::F),
            close_tab: i.modifiers.command && i.key_pressed(egui::Key::W),
            next_tab: i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Tab),
            prev_tab: i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Tab),
            print: i.modifiers.command && i.key_pressed(egui::Key::P),
        });

        if keys.escape && self.escape() == Escaped::ReturnedToPointer {
            ctx.memory_mut(|m| m.surrender_focus(command_id));
            let page = self.tab().page;
            if let Some(layer) = self.tab_mut().markup.existing_mut(page) {
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
        //
        // A shape or placed picture is tried first, and only while nothing
        // has focus — a text field with focus means ⌘C is meant for it, or
        // for `copy_selection`'s own text-selection path below, not for
        // whatever happens to be sitting selected on the page.
        if keys.copy && focus.allows_clipboard_keys(command_id) && self.copy_organize_selection() {
            // handled — pages were copied out of the thumbnail rail (or the
            // wider Organize grid — both share one selection, and
            // `copy_organize_selection` itself is a no-op when it's empty,
            // so this falls through to object/text copy exactly as before
            // whenever no page is selected).
        } else if keys.copy && focus.allows_clipboard_keys(command_id) && self.copy_object_selection() {
            // handled — an object was copied, not text.
        } else if keys.copy && self.copy_editing_run(ctx) {
            // handled — the run or paragraph open in the editor, copied whole
            // because nothing inside its box was selected.
        } else if keys.copy || std::mem::take(&mut self.tab_mut().copy_wanted) {
            self.copy_selection(ctx);
        }
        // See `clipboard_mirror_wanted`'s own doc comment: a page/object copy
        // has nothing to do with the *system* clipboard, but ⌘V's
        // `Event::Paste` only ever fires when that clipboard already holds
        // non-empty text, so this gives it something right after every
        // successful copy — the one place in the app that actually has the
        // `egui::Context` a real write needs.
        if std::mem::take(&mut self.clipboard_mirror_wanted) {
            ctx.copy_text(COPIED_IN_PAGIFY.to_string());
        }
        if keys.find_next && !self.tab_mut().find_hits.is_empty() {
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
        if keys.print {
            self.print_current_document(frame);
        }
        if keys.undo {
            self.undo_redo(true);
        }
        if keys.redo {
            self.undo_redo(false);
        }
        if keys.close_tab {
            self.close_tab(self.active_tab);
        }
        if keys.next_tab && self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        }
        if keys.prev_tab && self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
        }
        if keys.search {
            // Puts the cursor in the box with `find ` typed, which is the
            // nearest thing to a search field in an app whose interface is a
            // command line.
            self.cmd.input_mut().clear();
            self.cmd.input_mut().push_str("find ");
            self.command_open = true;
            ctx.memory_mut(|m| m.request_focus(command_id));
            self.caret_to_end_of_command_box(ctx, command_id);
        }

        if focus.allows_submit(command_id) {
            // The box is drawn after this, in the same frame, and a one-line
            // text field takes Up/Down as "caret to the start/end" — so the
            // key is consumed here, or it moves the caret again right after
            // the recall put it at the end of the recalled line.
            if keys.up {
                self.cmd.recall_previous();
                self.caret_to_end_of_command_box(ctx, command_id);
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
            }
            if keys.down {
                self.cmd.recall_next();
                self.caret_to_end_of_command_box(ctx, command_id);
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
            }
        }

        // Document keys. Deny by default — see focus.rs.
        if focus.allows_document_keys() && self.tab_mut().doc.is_some() {
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
                // Pages selected in the rail (or the wider Organize grid —
                // one selection, shared) take priority, the same precedence
                // copy already gives pages over objects above.
                if !self.tab_mut().organize_selected.is_empty() {
                    self.delete_organize_selection();
                } else {
                    self.delete_selection();
                }
            }
            // Enter closes a pick that has no fixed number of points — an area
            // measurement, or a polyline. `done` is the same thing typed.
            if keys.enter {
                let closeable = self.tab_mut()
                    .tool
                    .as_ref()
                    .is_some_and(|p| p.kind.ends_on_enter() && p.points.len() >= 2);
                if closeable {
                    self.resolve();
                }
            }
        }

        // Paste shares ⌘C's own clipboard-keys gate, not the strict
        // document-keys one above: it is one half of the same shortcut the
        // command box legitimately re-focuses itself after (see
        // Focus::allows_clipboard_keys), so it needs to keep working right
        // after a typed `copy`/`paste`, not just once focus is empty.
        if keys.paste && focus.allows_clipboard_keys(command_id) && self.tab_mut().doc.is_some() {
            let pasted = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
            });
            if self.current_page_clipboard().is_some() {
                self.paste_organize_selection();
            } else if !self.start_paste_ghost(pasted) {
                self.paste_object_selection();
            }
        }
    }

    /// A file dropped on the window, and a document Finder handed over.
    fn open_dropped_files(&mut self, ctx: &egui::Context) {
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
    }

    /// The title bar and its document tabs, including the deferred switch,
    /// close and strip-publishing that have to happen after the panel's own
    /// closure has ended.
    fn draw_title_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        // -- title bar --------------------------------------------------------
        // Document tabs share the titlebar's own row, right beside the logo
        // — the compact mockup's own layout (`pagify_pdf_compact_dark_theme/
        // code.html:108-130`), rather than a second row of their own. Always
        // shown, even with one tab open: a strip that pops in and out of
        // existence as tabs come and go is more surprising than one that's
        // just always there, one pill wide.
        let mut switch_to: Option<usize> = None;
        let mut close_clicked: Option<usize> = None;
        let mut tab_rects: Vec<egui::Rect> = Vec::new();
        // How many tabs the strip had room for this frame.
        let mut visible_tabs = usize::MAX;
        let title_bar = egui::Panel::top("titlebar")
            .frame(egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // The row is as tall as a document tab from the start, so
                    // the logo, the name and the checkboxes — placed before
                    // any tab — are centred on the line the tabs will be on.
                    ui.set_min_height(doc_tab_height(ui));
                    // Shrunk from 32/24pt toward the mockup's own compact
                    // mark — a titlebar logo does not need to compete with
                    // the document tabs now sharing its row for width.
                    let (rect, logo) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    // Not an `Image`: the tests that look for the page thumbnails
                    // look for the Image role, and the logo is not one.
                    logo.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Pagify logo"));
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
                            theme::icon_tile(ui.painter(), rect, theme::violet_bright(), theme::violet_deep());
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "Pa",
                                egui::FontId::proportional(10.0),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                    ui.add_space(5.0);
                    ui.label(
                        egui::RichText::new("Pagify")
                            .color(theme::ink())
                            .font(egui::FontId::proportional(15.0)),
                    );
                    ui.add_space(8.0);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // **The build number, at the far right of the bar** —
                        // asked for so that a tester's report can say which
                        // build it is about without anyone opening a log.
                        // First in a right-to-left row, so it is the rightmost.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("v{}", pagify_shell::VERSION))
                                    .size(11.0)
                                    .color(theme::ink_dim()),
                            )
                            .selectable(false),
                        )
                        .on_hover_text("The build of Pagify you are running.");
                        if self.tab_mut().doc.is_some() {
                            ui.checkbox(&mut self.ortho, "Ortho");
                            ui.checkbox(&mut self.show_thumbs, "Pages");
                        }

                        // **One row of tabs, the newest on the left.** Reported
                        // from use: with a dozen documents open the tabs
                        // wrapped onto five rows and took the page's space. The
                        // ones that fit are drawn; the rest are behind the small
                        // triangle, which sits just left of the checkboxes — in
                        // this layout the first thing placed after them.
                        let names: Vec<String> = self
                            .tabs
                            .iter()
                            .map(|tab| {
                                tab.doc
                                    .as_ref()
                                    .and_then(|d| d.session.path().file_name().map(|n| n.to_string_lossy().into_owned()))
                                    .unwrap_or_else(|| "Untitled".to_string())
                            })
                            .collect();
                        let widths: Vec<f32> = names.iter().map(|n| doc_tab_width(ui, n)).collect();
                        let visible = tabs_that_fit(&widths, ui.spacing().item_spacing.x, ui.available_width(), DOC_TAB_MENU_WIDTH);
                        visible_tabs = visible;
                        if visible < names.len() {
                            let menu = tab_menu_button(ui, names.len() - visible);
                            egui::Popup::menu(&menu).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                                ui.set_min_width(280.0);
                                for (i, name) in names.iter().enumerate().skip(visible) {
                                    let shown = fit_tab_label(ui, name, DOC_TAB_MAX_TEXT * 2.0);
                                    if ui.button(shown).on_hover_text(name).clicked() {
                                        switch_to = Some(i);
                                        ui.close();
                                    }
                                }
                            });
                        }

                        // The tabs fill whatever room is left between the
                        // logo and the checkboxes above — never wrapped, so
                        // the strip stays one row however many are open.
                        //
                        // **This `left_to_right` wrapper is not redundant, and
                        // taking it out put the tabs on the wrong side.**
                        // `horizontal_wrapped` does not always lay out left to
                        // right: it follows the direction of the layout it is
                        // inside, and that one is right-to-left (to keep the
                        // checkboxes at the far edge). Without this, the strip
                        // hugged the right of the bar *and* listed the tabs
                        // in reverse — "why are the tabs aligned to right?".
                        // Measured, not reasoned about: see
                        // `the_title_bar_reads_left_to_right_and_sits_on_one_line`.
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            // A wrapped row starts as tall as `interact_size`
                            // says and places its first widget at its own top —
                            // so a tab taller than that, in a row centred around
                            // it, sat lower than everything else on the line
                            // (measured: 38.0 against 32.5). Starting the row as
                            // tall as a tab fills the line instead.
                            ui.spacing_mut().interact_size.y = doc_tab_height(ui);
                            ui.horizontal(|ui| {
                                for (i, name) in names.iter().enumerate().take(visible) {
                                    let button = doc_tab_button(ui, name, i == self.active_tab, self.dragging_tab(i));
                                    tab_rects.push(button.response.rect);
                                    // A tab can be picked up and carried to
                                    // another window, or out into a window of
                                    // its own — see `hub`.
                                    self.tab_drag_event(ctx, i, &button, name);
                                    if button.select {
                                        switch_to = Some(i);
                                    }
                                    if button.close {
                                        close_clicked = Some(i);
                                    }
                                }
                            });
                        });
                    });
                });
            });
        self.publish_strip(ctx, title_bar.response.rect, tab_rects);
        // Deferred past the panel's own closure, same as every other
        // click-to-select in this file — the panel borrows `ui` for its own
        // duration, so nothing inside it can also call back into `self`.
        if let Some(i) = switch_to {
            self.active_tab = i;
        }
        // The tab showing is always one of the tabs in the strip: one chosen from
        // the menu, or left showing when another was closed, takes the place of
        // the last one that fits.
        if let Some(i) = close_clicked {
            self.close_tab(i);
        }
        self.bring_active_tab_into_the_strip(visible_tabs);
    }

    /// The ribbon, top panels and all. Returns the command a button asked to
    /// run, so the caller can run it after every panel has been drawn.
    fn draw_ribbon(&mut self, ui: &mut egui::Ui) -> Option<String> {
        // -- ribbon ----------------------------------------------------------
        let mut ribbon_command: Option<String> = None;
        egui::Panel::top("ribbon")
            .frame(egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ui, |ui| {
                // Wrapped, not scrolled. Sixteen tabs do not fit a narrow
                // window, and a tab that has scrolled out of sight is a tab
                // nobody knows is there — a second row is the cheaper cost.
                ui.horizontal_wrapped(|ui| {
                    for tab in Tab::ALL {
                        if tab_button(ui, tab.label(), self.tab_mut().ribbon == tab) {
                            self.tab_mut().ribbon = tab;
                        }
                    }
                });
            });

        egui::Panel::top("ribbon_actions")
            .frame(
                egui::Frame::new()
                    .fill(theme::paper())
                    .inner_margin(egui::Margin::symmetric(RIBBON_MARGIN_X, RIBBON_MARGIN_Y)),
            )
            .show(ui, |ui| {
                let tab = self.tab_mut().ribbon;

                // Tight horizontally, loose vertically. The buttons already
                // carry their own padding, so spacing between them only adds
                // gaps to a grid that reads better closed up — but a row that
                // wraps needs air between the rows or the two run together.
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 6.0);

                // Which tool is in force. The pointer mode, or whichever tool
                // is part-way through collecting its clicks — a user who armed
                // Line and looked away needs to see that it is still armed.
                let armed = self.tab_mut().tool.as_ref().and_then(|p| p.kind.command());
                let in_hand = self.tab_mut()
                    .markup_armed
                    .map(|k| match k {
                        pagify_shell::verbs::Markup::Highlight => "highlight",
                        pagify_shell::verbs::Markup::Underline => "underline",
                        pagify_shell::verbs::Markup::StrikeOut => "strikeout",
                        pagify_shell::verbs::Markup::Squiggly => "squiggly",
                    })
                    .or(match self.tab().object_tool {
                        Some(true) => Some("editobject"),
                        Some(false) => Some("moveobject"),
                        None => None,
                    });
                let show_hand_by_default =
                    self.tab().hand_shown_before_any_tool_is_picked
                        && self.tab().pointer == pagify_shell::verbs::PointerMode::Select;
                let live = |command: &str| -> bool {
                    let c = command.trim();
                    in_hand == Some(c)
                        || armed.as_deref() == Some(c)
                        || (c == "fill" && self.draw_fill)
                        || (c == "appearance" && theme::mode() == theme::Mode::Light)
                        || (c == "hand" && show_hand_by_default)
                        || match self.tab().pointer {
                            pagify_shell::verbs::PointerMode::Select => {
                                c == "selecttool" && !show_hand_by_default
                            }
                            pagify_shell::verbs::PointerMode::Pan => c == "hand",
                        }
                };

                // **One row, not a second one wrapped underneath — reported
                // from use, with a screenshot.** `horizontal_wrapped` used to
                // drop whatever did not fit onto a second row, permanently
                // visible and pushing the page down; requested instead as a
                // single row with the rest behind an arrow. `horizontal`
                // (not wrapped) plus a manual width check does that: once
                // the next button would not fit alongside room for the
                // arrow itself, everything from there on is held back from
                // the row.
                //
                // **What the arrow opens is the rest of the ribbon, not a
                // menu.** A plain text list of the held-back names was the
                // first version, and was refused: "just drop the rest of the
                // ribbon like how we had it". So it drops the same tiles
                // down, wrapped across the strip's own width, over the page
                // rather than shoving the page down to make room.
                const DROPDOWN_RESERVE: f32 = 30.0;
                const DIVIDER_WIDTH: f32 = 12.0;
                ui.horizontal(|ui| {
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
                    let group_starts = tab.button_group_starts();
                    let buttons = tab.buttons();
                    let slot_widths: Vec<f32> = (0..buttons.len())
                        .map(|i| TOOL_WIDTH + if group_starts.contains(&i) { DIVIDER_WIDTH } else { 0.0 })
                        .collect();
                    let overflow_from =
                        ribbon_overflow_at(ui.available_width(), &slot_widths, DROPDOWN_RESERVE);
                    for (i, (glyph, label, command)) in buttons.iter().enumerate() {
                        if i >= overflow_from {
                            break;
                        }
                        if group_starts.contains(&i) {
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }
                        let response = tool_button(ui, glyph, label, command, live(command));
                        // The one button on the ribbon that is a standing
                        // choice rather than a tool or an action: nothing
                        // about a small icon says what it means, or which way
                        // it is currently set, without this.
                        let response = if *command == "fill" {
                            response.on_hover_text(if self.draw_fill {
                                "Fill: on — the next rectangle or circle is drawn filled. \
                                 Click to draw hollow instead."
                            } else {
                                "Fill: off — the next rectangle or circle is drawn hollow. \
                                 Click to draw it filled instead."
                            })
                        } else if *command == "appearance" {
                            response.on_hover_text(if theme::mode() == theme::Mode::Light {
                                "Light theme — click for dark."
                            } else {
                                "Dark theme — click for light."
                            })
                        } else {
                            response
                        };
                        if response.clicked() {
                            // Every button runs a command string — §7.
                            ribbon_command = Some((*command).to_string());
                        }
                    }
                    let popup_id = egui::Id::new("ribbon_more_tools");
                    if overflow_from < buttons.len() {
                        let toggle = more_tools_button(ui, egui::Popup::is_id_open(ui.ctx(), popup_id));
                        // Anchored to the whole strip rather than to the small
                        // arrow, so the rest of the ribbon drops from the
                        // strip's own left edge and is as wide as the strip.
                        let strip = egui::Rect::from_min_max(
                            egui::pos2(
                                ui.max_rect().left() - f32::from(RIBBON_MARGIN_X),
                                toggle.rect.top() - f32::from(RIBBON_MARGIN_Y),
                            ),
                            egui::pos2(
                                ui.max_rect().right() + f32::from(RIBBON_MARGIN_X),
                                toggle.rect.bottom() + f32::from(RIBBON_MARGIN_Y),
                            ),
                        );
                        let shadow = ui.visuals().popup_shadow;
                        egui::Popup::from_toggle_button_response(&toggle)
                            .id(popup_id)
                            .anchor(strip)
                            .align(egui::RectAlign::BOTTOM_START)
                            // Never flipped above the strip: below it is the page,
                            // with room for however many rows this needs.
                            .align_alternatives(&[])
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .frame(
                                egui::Frame::new()
                                    .fill(theme::paper())
                                    .stroke(egui::Stroke::new(1.0, theme::line()))
                                    .shadow(shadow)
                                    .inner_margin(egui::Margin::symmetric(RIBBON_MARGIN_X, RIBBON_MARGIN_Y)),
                            )
                            .show(|ui| {
                                ui.set_width(strip.width() - 2.0 * f32::from(RIBBON_MARGIN_X));
                                ui.spacing_mut().item_spacing = egui::vec2(2.0, 6.0);
                                ui.horizontal_wrapped(|ui| {
                                    for (i, (glyph, label, command)) in
                                        buttons.iter().enumerate().skip(overflow_from)
                                    {
                                        if group_starts.contains(&i) {
                                            ui.add_space(4.0);
                                            ui.separator();
                                            ui.add_space(4.0);
                                        }
                                        if tool_button(ui, glyph, label, command, live(command)).clicked() {
                                            ribbon_command = Some((*command).to_string());
                                            egui::Popup::close_id(ui.ctx(), popup_id);
                                        }
                                    }
                                });
                            });
                    } else {
                        // A different tab, or a wider window, with nothing held
                        // back: a panel left open from before must not come back
                        // the next time something is.
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                });

            });
        ribbon_command
    }

    /// The command bar, bottom panel: the single line, or the opened box with
    /// its history above it. A submit lands in `submitted` for the caller to
    /// dispatch once the whole frame has been laid out.
    fn draw_command_bar(&mut self, ui: &mut egui::Ui, command_id: egui::Id, submitted: &mut Option<Dispatch>) {
        // -- command bar ------------------------------------------------------
        //
        // Two shapes: the single line the mockup draws, and an opened box with
        // the history above it. Resizable while open, because how much history
        // you want to see is not something this can know.
        let bar_frame = egui::Frame::new()
            .fill(theme::chrome())
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
                                Kind::Echo => (theme::ink_dim(), format!("› {}", entry.text)),
                                Kind::Info => (theme::ink(), entry.text.clone()),
                                Kind::Error => (theme::danger(), entry.text.clone()),
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
            let wants = match &self.tab_mut().tool {
                Some(armed) => armed.prompt(),
                None => self.cmd.prompt().wants.clone(),
            };

            if self.command_open {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&name)
                            .color(theme::violet_bright())
                            .font(egui::FontId::monospace(13.0)),
                    );
                    ui.colored_label(theme::ink_faint(), "›");
                    ui.colored_label(theme::ink_dim(), &wants);
                });
            }

            ui.horizontal(|ui| {
                if !self.command_open {
                    // The mockup's own status bar carries a couple of
                    // document stats beside the command line (`code.html:578
                    // -588`). Page count only, not its own searchable-
                    // character count too: that number comes from
                    // `Session::classify`, which walks the page's own
                    // content — fine once, when `textlayer`/`extracttext`
                    // already asks for it, but not something to recompute on
                    // every one of sixty frames a second for a passive
                    // readout nobody asked to see live.
                    if let Some(page_count) = self.tab_mut().doc.as_ref().map(|d| d.page_count) {
                        let page = self.tab_mut().page;
                        ui.colored_label(theme::ink_faint(), format!("page {} of {page_count}", page + 1));
                        ui.add_space(8.0);
                    }
                    ui.label(
                        egui::RichText::new(&name)
                            .color(theme::violet_bright())
                            .font(egui::FontId::monospace(13.0)),
                    );
                    ui.colored_label(theme::ink_faint(), ">");
                    // A tool that is waiting for clicks must say so even with
                    // the history folded away. Without this, arming a tool
                    // looked exactly like nothing happening.
                    if self.tab_mut().tool.is_some() {
                        // **An error that has just been said stays, with the
                        // prompt after it.** Reported from use: a click that
                        // found no text, or an apply that was refused, said
                        // so in red and was then covered by this very line in
                        // the same frame — nothing showed unless the history
                        // happened to be open. Only an error that is *still
                        // the last thing said* counts, so it goes the moment
                        // anything else is said, arming a tool again included.
                        let error = self
                            .cmd
                            .history()
                            .last()
                            .filter(|last| matches!(last.kind, Kind::Error))
                            .map(|last| last.text.clone());
                        match error {
                            Some(error) => {
                                let font = egui::FontId::proportional(12.0);
                                let mut job = egui::text::LayoutJob::default();
                                job.append(
                                    &error,
                                    0.0,
                                    egui::TextFormat::simple(font.clone(), theme::danger()),
                                );
                                job.append(
                                    &format!("   {wants}"),
                                    0.0,
                                    egui::TextFormat::simple(font, theme::snap()),
                                );
                                ui.add(egui::Label::new(job).truncate());
                            }
                            None => {
                                ui.colored_label(theme::snap(), &wants);
                            }
                        }
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
                            Kind::Error => (theme::danger(), last.text.as_str()),
                            Kind::Echo => (theme::ink_faint(), last.text.as_str()),
                            _ => (theme::ink_dim(), last.text.as_str()),
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
                // has, rather than jumping when the arrow changes direction.
                let toggle_width = 26.0;
                let input_width = (ui.available_width() - toggle_width - 8.0).max(80.0);

                let is_password = self.tab().awaiting_password.is_some();
                let command_open = self.command_open;
                let hint_text = match &self.tab().awaiting_password {
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
                    Some(Awaiting::Open(_)) if command_open => "type a command",
                    Some(Awaiting::Open(_)) => "type a command...",
                    None if command_open => "type a command",
                    None => "type a command...",
                };
                let response = ui.add_sized(
                    egui::vec2(input_width, 22.0),
                    egui::TextEdit::singleline(self.cmd.input_mut())
                        .id(command_id)
                        .font(egui::FontId::monospace(13.0))
                        .password(is_password)
                        .hint_text(hint_text),
                );
                if response.changed() {
                    self.cmd.note_edited();
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !self.consume_password_line() {
                        *submitted = self.cmd.submit(Submit::Enter);
                    }
                    response.request_focus();
                }

                if history_toggle(ui, egui::vec2(toggle_width, 22.0), self.command_open).clicked() {
                    self.command_open = !self.command_open;
                }
            });

            if self.command_open {
                ui.horizontal(|ui| {
                    if ui.button("Run").clicked()
                        && !self.consume_password_line()
                       
                    {
                        *submitted = self.cmd.submit(Submit::Button);
                    }
                    let page = self.tab().page;
                    let marks = self.tab().markup.existing(page).map(|l| l.len()).unwrap_or(0);
                    let page_count = self.tab().doc.as_ref().map(|d| d.page_count);
                    let calibrated = self.tab().calibration.is_calibrated();
                    let zoom_pct = self.resolved_zoom() * 100.0;
                    ui.small(match page_count {
                        Some(page_count) => format!(
                            "page {} of {}   ·   {:.0}%   ·   {marks} mark{}{}{}",
                            page + 1,
                            page_count,
                            zoom_pct,
                            if marks == 1 { "" } else { "s" },
                            if calibrated { "   ·   calibrated" } else { "" },
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
    }

    /// Everything below the command bar: the File backstage flag, the
    /// thumbnail rail or the Organize grid, the layers window, the properties
    /// panel, the page canvas — and the deferred handling of a rail jump, a
    /// ribbon/home command and a submitted command line.
    fn draw_main_area(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        command_id: egui::Id,
        ribbon_command: Option<String>,
        submitted: &mut Option<Dispatch>,
    ) {
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
        let backstage = self.tab_mut().ribbon == Tab::File;

        let jump_to = self.draw_left_rail(ctx, ui, backstage);

        self.draw_layers_window(ctx, backstage);

        // Claims its space before the central panel takes the rest — same
        // rule as the ribbon and the command bar above.
        self.draw_properties_panel(ui);

        let home_command = self.draw_page_canvas(ctx, ui, backstage, command_id);
        if let Some(page) = jump_to {
            self.act(Verb::Page(PageTarget::Number(page + 1)));
        }
        if let Some(command) = home_command.or(ribbon_command) {
            // The default-Hand illusion (see `hand_shown_before_any_tool_is_picked`)
            // only holds until the first real pick — from here on the ribbon
            // shows whichever tool is actually armed.
            self.tab_mut().hand_shown_before_any_tool_is_picked = false;
            match ribbon_click(&command) {
                // With nothing open a click that needs a document or its
                // pages has nothing to work on: say so, once and calmly,
                // rather than run it into a red usage error.
                RibbonClick::PickFile | RibbonClick::Extract | RibbonClick::Fill(Some(_))
                    if self.tab().doc.is_none() =>
                {
                    self.say_info("open a PDF first.");
                }
                RibbonClick::PickFile => self.import_pages_dialog(),
                RibbonClick::Extract => self.open_extract_dialog(),
                // Put it in the box rather than running it silently, so the
                // user sees the words the button stands for — which is the
                // whole claim.
                RibbonClick::Run => {
                    self.cmd.input_mut().clear();
                    self.cmd.input_mut().push_str(&command);
                    *submitted = self.cmd.submit(Submit::Button);
                }
                RibbonClick::Fill(usage) => {
                    self.cmd.input_mut().clear();
                    self.cmd.input_mut().push_str(command.trim_end());
                    self.cmd.input_mut().push(' ');
                    if let Some(usage) = usage {
                        self.say_info(usage);
                    }
                    ctx.memory_mut(|m| m.request_focus(command_id));
                    self.caret_to_end_of_command_box(ctx, command_id);
                }
            }
        }
        if let Some(dispatch) = submitted.take() {
            // A typed line cancels any half-collected pick. Letting it swallow
            // the click silently would mean an unrelated command finishing
            // someone else's measurement.
            if self.tab_mut().tool.take().is_some() {
                self.say_info("that pick was cancelled.");
            }
            let line = self
                .cmd
                .history()
                .last()
                .map(|e| e.text.clone())
                .unwrap_or_default();
            self.recorder.observe(&line);
            self.session_log.record("command", &line);
            self.run(dispatch);
        }
    }

    /// The thumbnail rail or the Organize grid, whichever is showing.
    /// Returns a page the reader asked to jump to — deferred to the caller,
    /// because the panel borrows `ui` for its own duration.
    fn draw_left_rail(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, backstage: bool) -> Option<usize> {
        // -- thumbnails ----------------------------------------------------------
        let mut jump_to = None;
        // Nothing open means nothing to thumbnail — an empty rail is a strip of
        // furniture that does not do anything.
        if self.organize_open && !backstage && self.tab_mut().doc.is_some() {
            self.draw_organize_grid(ctx, ui);
        } else if self.show_thumbs && !backstage && self.tab_mut().doc.is_some() {
            let modifiers = ctx.input(|i| i.modifiers);
            let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
            let released = ctx.input(|i| i.pointer.any_released());
            let mut cell_rects: Vec<(usize, egui::Rect)> = Vec::new();
            let mut import_pages = false;
            egui::Panel::left("thumbs")
                .resizable(true)
                .default_size(148.0)
                .size_range(104.0..=420.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::paper())
                        .inner_margin(egui::Margin::symmetric(8, 8)),
                )
                .show(ui, |ui| {
                // The rail's own header row — view-switcher icons on the
                // left (today, just the one view this rail has; `Bookmark`
                // sits beside it rather than off in the ribbon, matching the
                // mockup's own rail header, `code.html:296-322`), a collapse
                // chevron on the right so hiding the rail doesn't require
                // hunting for the titlebar's own "Pages" checkbox.
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("\u{E9B0}").font(icon_font(16.0)).color(theme::violet()),
                        )
                        .selectable(false),
                    )
                    .on_hover_text("Thumbnails");
                    if ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new("\u{E8E7}").font(icon_font(16.0)).color(theme::ink_dim()),
                            )
                            .selectable(false)
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Bookmarks")
                        .clicked()
                    {
                        self.toggle_bookmark_panel();
                    }
                    // Pages from another PDF, put into this one. The file
                    // dialog opens after the panel is drawn, not inside it.
                    let import = ui.add(
                        egui::Label::new(
                            egui::RichText::new("\u{E2C8}").font(icon_font(16.0)).color(theme::ink_dim()),
                        )
                        .selectable(false)
                        .sense(egui::Sense::click()),
                    );
                    import.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Insert pages from another PDF")
                    });
                    if import.on_hover_text("Insert pages from another PDF").clicked() {
                        import_pages = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // A plain Unicode character, not the custom icon
                        // font — same choice the command bar's own
                        // expand/collapse chevron already makes, and for the
                        // same reason: this needs to be in whatever font
                        // backs it with certainty, not a guess at the icon
                        // font's own Private Use Area coverage.
                        if ui
                            .add(
                                egui::Label::new(egui::RichText::new("‹").size(16.0).color(theme::ink_dim()))
                                    .selectable(false)
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Hide the Pages rail")
                            .clicked()
                        {
                            self.show_thumbs = false;
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let count = self.tab_mut().doc.as_ref().map(|d| d.page_count).unwrap_or(0);
                    for page in 0..count {
                        ui.vertical_centered(|ui| {
                            let width = ui.available_width();
                            if let Some(page) =
                                self.draw_thumbnail_cell(ctx, ui, page, modifiers, &mut cell_rects, width)
                            {
                                jump_to = Some(page);
                            }
                        });
                        ui.add_space(6.0);
                    }
                    Self::draw_drop_indicator(
                        ui,
                        self.tab_mut().organize_drag.is_some(),
                        &cell_rects,
                        pointer_pos,
                    );
                });
            });
            self.finish_thumbnail_drag(&cell_rects, pointer_pos, released);
            if import_pages {
                self.import_pages_dialog();
            }
        } else if !backstage && self.tab_mut().doc.is_some() {
            // **Reported from use: "once the thumbnail is hidden there's no
            // way to bring it back."** The collapse chevron above only ever
            // relied on the titlebar's own "Pages" checkbox for the way
            // back — but an *unchecked* checkbox has no visible outline in
            // this theme's flat style (`widgets.inactive.bg_stroke` is
            // `Stroke::NONE`, see theme.rs), so it reads as plain text, not
            // as something to click. This puts the way back in the one place
            // a reader's eye actually goes looking for it: exactly where the
            // rail used to be.
            egui::Panel::left("thumbs_collapsed")
                .resizable(false)
                .default_size(18.0)
                .frame(egui::Frame::new().fill(theme::paper()))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);
                        if ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new("\u{203A}").size(16.0).color(theme::ink_dim()),
                                )
                                .selectable(false)
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Show the Pages rail")
                            .clicked()
                        {
                            self.show_thumbs = true;
                        }
                    });
                });
        }
        jump_to
    }

    /// The floating Layers window, and the restack/opacity changes it asked
    /// for — applied once the window's own closure has ended.
    fn draw_layers_window(&mut self, ctx: &egui::Context, backstage: bool) {
        // -- layers ------------------------------------------------------------
        //
        // **A floating window, not a rail.** Asked for from use as a popup
        // with the order in it and the means to move things up and down — and
        // a window can be dragged next to the thing being re-ordered, where a
        // rail on the far side of the page cannot.
        let mut restack_to: Option<(usize, pdf_core::document::Stacking)> = None;
        let mut opacity_to: Option<(usize, f32)> = None;
        if self.show_layers && !backstage && self.tab_mut().doc.is_some() {
            let page = self.tab_mut().page;
            let picked = self.tab_mut().picked_layer;
            let entries: Vec<pdf_core::document::DrawnObject> = self.layers_on(page).to_vec();
            let mut pick: Option<usize> = None;
            let chosen_entry = picked.and_then(|at| entries.get(at));
            let armed = chosen_entry.is_some();
            let grouped = chosen_entry.is_some_and(|e| !e.movable);
            let mut open = true;

            egui::Window::new(format!("Layers — page {}", page + 1))
                .id(egui::Id::new("layers-window"))
                .open(&mut open)
                .default_size(egui::vec2(300.0, 380.0))
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    // Said once, here, because "later is on top" is the one fact
                    // that makes the list make sense and nothing else on screen
                    // says it.
                    ui.small("Topmost first — what is listed above covers what is below.");
                    ui.add_space(4.0);

                    if entries.is_empty() {
                        ui.label("Nothing this page draws could be listed.");
                        return;
                    }

                    let object = chosen_entry.map(|e| e.object);
                    ui.horizontal(|ui| {
                        use pdf_core::document::Stacking;
                        for (glyph, tip, to) in [
                            ("\u{E5D8}", "Move up one", Stacking::Up),
                            ("\u{E5DB}", "Move down one", Stacking::Down),
                            ("\u{E883}", "Bring to front", Stacking::Front),
                            ("\u{E882}", "Send to back", Stacking::Back),
                        ] {
                            let button = egui::Button::new(
                                egui::RichText::new(glyph).font(icon_font(18.0)),
                            )
                            .min_size(egui::vec2(34.0, 28.0));
                            if ui.add_enabled(armed, button).on_hover_text(tip).clicked() {
                                if let Some(object) = object {
                                    restack_to = Some((object, to));
                                }
                            }
                        }
                    });
                    if grouped {
                        ui.small(if chosen_entry.is_some_and(|e| e.label == "placeholder") {
                            "The picture's placeholder — it moves with the picture."
                        } else {
                            "Drawn inside a group — the whole group moves."
                        });
                    } else if !armed {
                        ui.small("Pick a row, or click something on the page with Edit Object.");
                    }

                    // **Opacity, applied when the slider is let go** — not on
                    // every frame of the drag, which would rewrite the page
                    // sixty times a second.
                    if let Some(entry) = chosen_entry.filter(|e| e.movable) {
                        let mut alpha = self.tab_mut().opacity_draft.unwrap_or(entry.opacity);
                        ui.add_space(6.0);
                        let slider = ui.add(
                            egui::Slider::new(&mut alpha, 0.0..=1.0)
                                .text("opacity")
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                                .custom_parser(|t| t.trim_end_matches('%').parse::<f64>().ok().map(|p| p / 100.0)),
                        );
                        if slider.changed() {
                            self.tab_mut().opacity_draft = Some(alpha);
                        }
                        if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
                            if (alpha - entry.opacity).abs() > 0.005 {
                                opacity_to = Some((entry.object, alpha));
                            }
                            self.tab_mut().opacity_draft = None;
                        }
                    }
                    ui.add_space(6.0);
                    ui.separator();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        // Drawn last is on top, so the list reads the other way
                        // round from the page's own order.
                        for (at, entry) in entries.iter().enumerate().rev() {
                            let chosen = picked == Some(at);
                            let row = ui
                                .horizontal(|ui| {
                                    // What a group draws is stepped in under it,
                                    // so a page laid out as one panel reads as
                                    // the panel and its contents rather than as
                                    // a single unreadable entry.
                                    ui.add_space(entry.depth as f32 * 14.0);
                                    ui.selectable_label(
                                        chosen,
                                        format!(
                                            "{}  {}",
                                            match entry.kind {
                                                pdf_core::document::DrawnKind::Words => "\u{E262}",
                                                pdf_core::document::DrawnKind::Picture => "\u{E3F4}",
                                                pdf_core::document::DrawnKind::Shape => "\u{E3C6}",
                                                pdf_core::document::DrawnKind::Group => "\u{E2C7}",
                                            },
                                            entry.label
                                        ),
                                    )
                                })
                                .inner;
                            if row.clicked() {
                                pick = Some(at);
                            }
                            row.on_hover_text(format!(
                                "{} — {:.0} × {:.0} pt at {:.0}, {:.0}{}",
                                entry.kind.describe(),
                                entry.rect.right - entry.rect.left,
                                entry.rect.bottom - entry.rect.top,
                                entry.rect.left,
                                entry.rect.top,
                                if entry.movable {
                                    ""
                                } else if entry.label == "placeholder" {
                                    "\nthe picture's placeholder — moves with the picture"
                                } else {
                                    "\ndrawn inside a group — the group is what moves"
                                },
                            ));
                        }
                    });
                });

            if !open {
                self.show_layers = false;
            }
            if let Some(at) = pick {
                self.tab_mut().picked_layer = Some(at);
            }
        }
        if let Some((object, where_to)) = restack_to {
            let page = self.tab_mut().page;
            match self.restack(page, object, where_to) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
        }
        if let Some((object, alpha)) = opacity_to {
            let page = self.tab_mut().page;
            let kept = self.tab_mut().picked_layer;
            match self.set_opacity_of(page, object, alpha) {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
            // The page was rewritten, but nothing moved: the same row is the
            // same thing.
            self.tab_mut().picked_layer = kept;
        }
    }

    /// The page canvas: the central panel, which is also where the Home
    /// wizard and the File backstage are drawn. Returns the command the
    /// wizard asked for.
    fn draw_page_canvas(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        backstage: bool,
        command_id: egui::Id,
    ) -> Option<String> {
        // -- the pages ---------------------------------------------------------
        let mut home_command: Option<String> = None;
        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.tab_mut().canvas_pt = ui.available_size();

            // **The command box's own placeholder says "type a command" —
            // make that literally true from the first frame.** Nothing on
            // either "nothing open" screen (the File-tab backstage a fresh
            // launch lands on, or this Home tab) takes keyboard focus by
            // default, so typing right after launch went nowhere: not even
            // into the box, just silently discarded, with no widget to
            // route it to. Reported from use as `status` "trying to open a
            // file" — what actually happened was a click aimed at finding
            // somewhere to type landing on a button instead, because
            // nothing told the thin command bar apart from the rest of an
            // empty screen. Only when nothing has already claimed focus: a
            // real click anywhere else must still win.
            if self.tab_mut().doc.is_none() && ctx.memory(|m| m.focused()).is_none() {
                ctx.memory_mut(|m| m.request_focus(command_id));
            }

            if backstage {
                // **Its own id, not the page strip's.** Both scroll areas are
                // made on this panel's ui, and egui keeps a scroll position
                // under the area's id alone — with the default one for both,
                // a frame of File stored a zero over where the reader was in
                // the document, and coming back landed on the first page.
                // Reported from use as the view jumping after Save or Open.
                let chosen = egui::ScrollArea::vertical()
                    .id_salt("backstage")
                    .auto_shrink([false, false])
                    .show(ui, |ui| home::show(ui, &self.recent, &self.outlined_fonts))
                    .inner;
                if let Some(command) = chosen {
                    home_command = Some(command);
                }
                return;
            }

            if self.tab_mut().doc.is_none() {
                // Not the wizard. This tab has nothing to show because there is
                // nothing open, and it should say so rather than quietly
                // becoming a different tab.
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);
                        ui.colored_label(theme::ink_faint(), "No document open.");
                        ui.add_space(10.0);
                        if ui.button("Open a PDF…").clicked() {
                            home_command = Some("open".to_string());
                        }
                        ui.add_space(6.0);
                        ui.small(
                            egui::RichText::new("or drop one on the window")
                                .color(theme::ink_faint()),
                        );
                    });
                });
                return;
            }

            self.draw_pages(ui, ctx, command_id);
        });
        home_command
    }


    fn new(path: Option<&str>) -> Self {
        Self::build(path, false)
    }

    /// `extra` is a window made after the first — see [`hub`]. It borrows
    /// the libraries the first one loaded (`Shared`, lent to whichever window is
    /// being drawn) instead of reading its own copy of each, which would be a
    /// stale copy the moment the other window changed anything and then saved
    /// over it.
    fn build(path: Option<&str>, extra: bool) -> Self {
        let quiet = cfg!(test) || extra;
        // The person's own words, shared by every window. Never under test: a
        // test run must not read or write the real dictionary.
        if !quiet {
            spelling::use_dictionary_file();
        }
        let mut app = PagifyApp {
            tabs: vec![DocTab::new()],
            active_tab: 0,
            cmd: CommandBox::default(),
            recorder: Recorder::default(),
            // **Nothing of the person's own under test.** The test suite
            // constructs hundreds of apps, and every one of them was writing
            // its fixture into the real Recent Documents list and reading the
            // real font list — reported as a recents list full of test files.
            recent: if quiet { Recent::default() } else { Recent::load() },
            outlined_fonts: if quiet {
                pagify_shell::outlined_fonts::OutlinedFonts::default()
            } else {
                pagify_shell::outlined_fonts::OutlinedFonts::load()
            },
            defaults: tools::Defaults::default(),
            draw_fill: false,
            mark: None,
            signature_textures: std::collections::HashMap::new(),
            snaps: SnapSet::defaults(),
            ortho: false,
            grid_pt: 0.0,
            show_thumbs: true,
            organize_open: false,
            show_layers: false,
            command_open: false,
            errors_said: 0,
            recogniser: None,
            signatures: if quiet {
                pagify_shell::signatures::Signatures::default()
            } else {
                pagify_shell::signatures::Signatures::load()
            },
            signatures_path: if cfg!(test) { None } else { pagify_shell::signatures::Signatures::path() },
            scripts_dir: if cfg!(test) {
                None
            } else {
                pagify_shell::state::state_dir().map(|d| d.join("scripts"))
            },
            pad: None,
            signature_list: None,
            predefined: if quiet {
                pagify_shell::predefined::Predefined::default()
            } else {
                pagify_shell::predefined::Predefined::load()
            },
            predefined_path: if cfg!(test) { None } else { pagify_shell::predefined::Predefined::path() },
            snippets: None,
            editor_face: None,
            editor_face_ready: false,
            editor_face_metrics: None,
            editor_face_coverage: None,
            pending_face: None,
            text_to_offer: None,
            face_cache: FaceCache::default(),
            system_fonts: None,
            font_picker_open: false,
            font_picker_filter: String::new(),
            // Real disk I/O under the user's actual config directory — a test
            // run must not litter it with hundreds of near-empty session
            // logs, the same reason `predefined` above skips its own real
            // load in `cfg!(test)`.
            session_log: if quiet {
                pagify_shell::session_log::SessionLog::default()
            } else {
                pagify_shell::session_log::SessionLog::start()
            },
            object_clipboard: None,
            paste_ghost: None,
            paste_count: 0,
            page_clipboard: None,
            clipboard_dir: if cfg!(test) {
                static APPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                std::env::temp_dir().join(format!(
                    "pagify-clipboard-test-{}-{}",
                    std::process::id(),
                    APPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ))
            } else {
                Self::shared_clipboard_dir()
            },
            clipboard_mirror_wanted: false,
            renders: None,
            async_render: !cfg!(test),
            render_stats: RenderStats::default(),
            handover: instance::Handover::default(),
            win: hub::WindowState::default(),
            replay_depth: 0,
            update_check: None,
            update_available: None,
            pending_update: None,
        };
        app.say_info(format!("Pagify {} — type `help`, or `open <path.pdf>`.", pagify_shell::VERSION));
        // `None` in a test run — `session_log` is a no-op there — so this
        // never adds a line the existing tests asserting on `history()`
        // would have to account for.
        if let Some(log_path) = app.session_log.path() {
            app.say_info(format!(
                "recording this session to {} — `sessionlog` any time for this path.",
                log_path.display()
            ));
        }
        if let Some(path) = path {
            app.open(path);
        }
        // Nothing to look at means starting backstage, where the wizard and the
        // recent documents are. `open` moves us to Home.
        if app.tab().doc.is_none() {
            app.tab_mut().ribbon = Tab::File;
        }
        // Never in a test — no test may depend on, or be slowed by, a real
        // filesystem check of a real Dropbox folder. Nor in a second window:
        // the update is offered once, by the first.
        if !quiet {
            app.spawn_update_check();
        }
        // Same reasoning as `recent`/`session_log` above: a test run must not
        // read the real, developer-machine appearance preference, or every
        // test constructed after a manual Light-mode session would silently
        // start in it. (A second window has it already: it is one setting for
        // the whole program.)
        if !quiet {
            theme::load_persisted_mode();
        }
        app
    }

    /// Put the caret at the end of what the command box now holds. Call it
    /// after **every** programmatic write to the box — a ribbon prefill,
    /// Ctrl+F's `find `, a recalled line.
    ///
    /// egui keeps the caret of a text field as a stored index, clamped to the
    /// text and never reset, and a submitted command leaves it at 0 in the
    /// emptied box. So a line written into the box afterwards sat *behind* the
    /// caret and what was typed next landed in front of it: Swap, then `1 2`,
    /// gave `1 2swappages `. Only a line typed in a box that had never held a
    /// command got the end by default, which is why a fresh test never saw it.
    fn caret_to_end_of_command_box(&self, ctx: &egui::Context, command_id: egui::Id) {
        let mut state = egui::TextEdit::load_state(ctx, command_id).unwrap_or_default();
        let end = self.cmd.input().chars().count();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
        state.store(ctx, command_id);
    }

    // -- document -----------------------------------------------------------

    /// Move the view by a drag, in screen pixels.
    ///
    /// Dragging the paper moves the paper, so the offset goes the other way.
    fn pan(&mut self, by: egui::Vec2) {
        let total = self.tab_mut().pan_by.unwrap_or(egui::Vec2::ZERO) - by;
        self.tab_mut().pan_by = Some(total);
    }

    /// Ask what to do about unsaved marks, and do it.
    ///
    /// Three ways out, all of them reachable with the mouse. **Discarding is
    /// one of them.** A guard that only offers "save" or "go back" does not
    /// protect anybody: work gets abandoned on purpose all the time — a mark
    /// put down to measure something, a line drawn to check a distance — and a
    /// program that will not let go of it is a program you have to kill.
    fn ask_about_unsaved(&mut self, ctx: &egui::Context) {
        let Some(intent) = self.tab_mut().closing.clone() else { return };
        let password_waiting = self.unsaved_password();
        let edited = self.unsaved_edits();
        let (marks, pages) = self.unsaved().unwrap_or((0, 0));
        if marks == 0 && !password_waiting && !edited {
            // Saved out from under the dialog — carry on with what was asked.
            self.tab_mut().closing = None;
            self.finish_closing(intent, ctx);
            return;
        }
        let mut decision: Option<Decision> = None;
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(match intent {
                Closing::Document | Closing::Tab(_) => "Close this document?",
                // Closing one of several windows is not quitting.
                Closing::Program if self.win.closing_leaves == hub::Leaving::Window && self.win.others > 0 => {
                    "Close this window?"
                }
                Closing::Program => "Quit Pagify?",
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
                self.tab_mut().closing = None;
                // A quit that was walking the windows stops here: see
                // `hub::plan`.
                self.win.quit_cancelled = true;
                self.say_info("still open.");
            }
            Some(Decision::Save) => {
                self.tab_mut().closing = None;
                if self.save(None) == SaveOutcome::Asking {
                    // The password question has taken over; closing waits
                    // until that is answered and can be asked again.
                    return;
                }
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
                self.tab_mut().closing = None;
                self.finish_closing(intent, ctx);
            }
        }
    }

    fn finish_closing(&mut self, intent: Closing, _ctx: &egui::Context) {
        match intent {
            Closing::Document => {
                if self.tab_mut().doc.take().is_some() {
                    self.tab_mut().markup.clear();
                    self.forget_passcode();
                    self.cmd.prompt_mut().document = None;
                    self.tab_mut().ribbon = Tab::File;
                    self.say_info("closed.");
                }
            }
            Closing::Tab(index) => {
                if self.tabs.len() == 1 {
                    // The window's own last tab: the window goes with it.
                    self.leave(hub::Leaving::Window);
                    return;
                }
                self.remove_tab(index);
            }
            Closing::Program => {
                // The tab just asked about is resolved (saved or discarded)
                // — gone from the strip like any other closed tab. If
                // another still has unsaved work, ask about that one next:
                // the same modal, one tab at a time, never a "save all".
                //
                // The last tab is **not** taken out: the window is leaving with
                // it, and the rest of this frame still draws a window that has a
                // tab. (A discarded tab still counts as unsaved, so taking it
                // out is also what stops it being asked about again.)
                if self.tabs.len() > 1 {
                    self.remove_tab(self.active_tab);
                    if let Some(next) = self.tab_with_unsaved_work() {
                        self.active_tab = next;
                        self.tab_mut().closing = Some(Closing::Program);
                        return;
                    }
                }
                self.leave(self.win.closing_leaves);
            }
        }
    }

    /// Whether tab `index` would lose work if it closed right now —
    /// `close_tab` and the multi-tab quit walk both need to ask this about a
    /// tab that isn't necessarily the one on screen.
    fn would_lose_work_in_tab(&mut self, index: usize) -> bool {
        let saved = self.active_tab;
        self.active_tab = index;
        let lost = self.would_lose_work();
        self.active_tab = saved;
        lost
    }

    /// The first tab with unsaved work, in tab order starting from whichever
    /// is active — so quitting asks about the tab already on screen before
    /// any other. `None` once every tab is safe to lose.
    fn tab_with_unsaved_work(&mut self) -> Option<usize> {
        let start = self.active_tab;
        let count = self.tabs.len();
        (0..count).map(|offset| (start + offset) % count).find(|&i| self.would_lose_work_in_tab(i))
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
        let Some(doc) = self.tab().doc.as_ref() else {
            self.say_error("nothing open.");
            return;
        };
        let session = doc.session.clone();
        let page_count = doc.page_count;
        if let Some(busy) = self.tab().reading.as_ref() {
            self.say_error(format!("already reading page {}.", busy.at + 1));
            return;
        }

        // No range means the page in front of you. That is what a button press
        // means, and a button that silently read 149 pages would be a trap.
        let queue: Vec<usize> = if spec.trim().is_empty() {
            vec![self.tab().page]
        } else {
            match pagify_shell::organize::parse_range(spec, page_count) {
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
        self.tab_mut().reading =
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
            if self.tab_mut().reading.is_none() {
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
        let Some(reading) = &self.tab_mut().reading else { return };

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
                let done = self.tab_mut().reading.take().expect("checked above");
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
                let Some(doc) = &self.tab_mut().doc else { return };
                // Written here, on the UI thread: it is an edit, it goes
                // through `execute` for undo, and it invalidates caches the UI
                // owns.
                match doc.session.execute(pdf_core::command::Command::AddTextLayer {
                    page_index: page,
                    words: reading.words,
                }) {
                    Ok(_) => {
                        if let Some(r) = &mut self.tab_mut().reading {
                            r.words += words;
                            r.pages += 1;
                        }
                        if let Some(doc) = &mut self.tab_mut().doc {
                            doc.rendered_is_stale();
                        }
                        self.tab_mut().text_selection = None;
                        self.tab_mut().find_hits.clear();
                    }
                    Err(e) => self.say_error(format!("page {}: {e}", page + 1)),
                }
            }
        }

        // Move the progress line on, and let the next frame collect the next
        // page.
        if let Some(r) = &mut self.tab_mut().reading {
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

    /// Look for a newer build, without holding the window up.
    ///
    /// `UPDATE_FOLDER` may be a Dropbox mount that is not syncing, on a
    /// machine that does not have Dropbox at all, or simply absent — all of
    /// that is a plain filesystem read away, but "plain" still means it can
    /// stall (a network drive gone quiet) or fail, and neither is allowed to
    /// delay the window appearing. Same shape as `extract_text` above: a
    /// thread, a channel, a per-frame poll.
    fn spawn_update_check(&mut self) {
        let current = pagify_shell::VERSION.to_string();
        let (tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let found = read_update_manifest(std::path::Path::new(UPDATE_FOLDER))
                .filter(|candidate| newer_version(&current, candidate))
                .map(|version| (version, std::path::PathBuf::from(UPDATE_FOLDER)));
            let _ = tx.send(found);
        });
        self.update_check = Some(UpdateCheck { done });
    }

    /// Collect the update check, if it has answered.
    fn collect_update_check(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.update_check else { return };
        match check.done.try_recv() {
            Ok(found) => {
                self.update_check = None;
                self.update_available = found;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_check = None;
            }
        }
    }

    /// A file asked for a password; the next line typed is it.
    ///
    /// Intercepted **before** the command box sees it, which is the whole
    /// point: `CommandBox::submit` echoes every line into the visible history
    /// and the recorder keeps it for replay. A password must reach PDFium and
    /// nothing else — not the history, not the recording, not the recent list.
    ///
    /// Returns whether the line was taken.
    ///
    /// **The guard is `is_none`, not `is_some`.** Nothing is being asked for
    /// most of the time, and that is exactly when this must do nothing and
    /// leave the box alone — an inverted guard here cleared and swallowed
    /// every ordinary command's Enter, silently, because `CommandBox::submit`
    /// never got to see what had been typed. Found live, not in review: no
    /// test exercises a real keystroke-and-Enter through this box (every
    /// existing test calls the `submit(&str)` shortcut below, which never
    /// passes through here at all), so nothing caught it until someone typed
    /// `status` and pressed Enter and nothing happened.
    fn consume_password_line(&mut self) -> bool {
        // Opening, locking and securing all still ask here rather than in a
        // window of their own — see the match below. Only when *nothing* is
        // being asked must this leave the box alone, or a password prompt
        // still open at all is a plain command that was actually meant for a
        // window somewhere.
        if self.tab_mut().awaiting_password.is_none() {
            return false;
        }
        let typed = self.cmd.input_mut().trim().to_string();
        if typed.is_empty() {
            return false;
        }
        self.cmd.input_mut().clear();

        // `take()`, not `.expect()` on it: the check above and this line are
        // not the same instant, and egui can re-run the widget that reads
        // this within one logical update — found live, not in review, when
        // typing an ordinary command crashed here. Nothing left to take
        // means nothing was actually being asked for any more; fall through
        // exactly as if this function had never been called.
        let Some(awaiting) = self.tab_mut().awaiting_password.take() else {
            return false;
        };
        match awaiting {
            Awaiting::Open(path) => self.open_with(&path, Some(&typed)),
            // `answer_lock_passcode` already handles every one of these — and,
            // for `Unlock`/`UnlockItem`, remembers a passcode that worked the
            // same way locking one already did. Reported from use: typing the
            // one password a document was opened with, then being asked for
            // it again for every single passage on the page it had sealed —
            // this duplicated match once had none of that, `answer_lock_passcode`'s
            // copy had it only for the three lock variants, and the two had
            // quietly drifted apart. One dispatch now, not two to keep in step.
            other @ (Awaiting::Lock { .. }
            | Awaiting::LockPages(_)
            | Awaiting::LockImage { .. }
            | Awaiting::UnlockItem(_)
            | Awaiting::Unlock) => {
                self.tab_mut().awaiting_password = Some(other);
                self.answer_lock_passcode(&typed);
            }
            Awaiting::LockAgain { first, then } => {
                if typed != first {
                    self.say_error("those did not match — nothing was locked. Try again.");
                } else {
                    self.tab_mut().awaiting_password = Some(*then);
                    // The passcode is already known to be right; hand it on to
                    // whichever lock was waiting for it.
                    self.answer_lock_passcode(&typed);
                }
            }
            Awaiting::SecureCurrent(options) => {
                // Handled in `answer_passcode`; the command box no longer takes
                // passwords, so this only exists to keep the match whole.
                self.tab_mut().awaiting_password = Some(Awaiting::SecureCurrent(options));
            }
            Awaiting::Certificate(path) => {
                self.tab_mut().awaiting_password = Some(Awaiting::Certificate(path));
            }
            Awaiting::Secure(options) => {
                self.tab_mut().awaiting_password =
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
        }
        true
    }



    /// Ask for a file.
    ///
    /// Blocks the frame while the dialog is up, which is what a modal open
    /// dialog is. Doing it asynchronously would mean holding a half-open
    /// document across frames for no gain — nobody expects to keep working in
    /// the window behind an open dialog.
    /// Marks that closing right now would discard, and the pages they are on.
    fn unsaved(&self) -> Option<(usize, usize)> {
        if self.tab().markup.revision() == self.tab().saved_revision {
            return None;
        }
        let (marks, pages) = self.tab().markup.unsaved();
        (marks > 0).then_some((marks, pages))
    }

    /// Sign the document with a certificate.
    fn sign_with(&mut self, path: &std::path::Path, password: &str) -> Result<String, String> {
        let pkcs12 = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };

        let who = doc
            .session
            .sign_document(&pkcs12, password, &pdf_core::pdf::sign::Reason::default())
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        // Says the two things somebody will otherwise learn the hard way:
        // the signature is not on disk until a save writes it, and it covers
        // the file as it now stands — an edit after this is outside it, and
        // the check will say so.
        Ok(format!(
            "signed as {who} — save to write it out. The signature covers the file \
             exactly as it is now; anything changed after this is outside it, and \
             `validate` will say the document was changed after it was signed."
        ))
    }

    /// Everything this document's protections amount to, in a few lines.
    ///
    /// **What it deliberately does not do is go looking.** A hidden-data survey
    /// reads the whole file, and a status readout somebody presses out of
    /// curiosity on an 80 MB catalogue must not stop to do that. It says which
    /// tool looks, instead of pretending it already has.
    fn document_status(&self) -> Vec<String> {
        let Some(doc) = &self.tab().doc else { return vec!["nothing open.".into()] };
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
                    lines.push(signature_line("signed", signature));
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

    /// Keep an uploaded picture as a signature — the image counterpart of
    /// [`Self::save_drawn_signature`], and the same contract: a name, the
    /// same "kept on this computer" line, the same undo-on-failure.
    fn save_uploaded_signature(
        &mut self,
        name: &str,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Result<String, String> {
        let name = match name.trim() {
            "" => "Signature".to_string(),
            given => given.to_string(),
        };
        let Some(signature) = pagify_shell::signatures::Signature::from_image(&name, rgba, width, height)
        else {
            return Err("that picture has no size, or its pixels do not match it.".into());
        };

        self.signatures.add(signature);
        if let Err(e) = self.keep_signatures() {
            self.signatures.remove(&name);
            return Err(e);
        }
        Ok(format!(
            "kept \"{name}\" on this computer{}. `signature` places it.",
            match &self.signatures_path {
                Some(path) => format!(", in {}", path.display()),
                None => String::new(),
            }
        ))
    }

    /// A name to offer for an uploaded picture: the file's own name, since
    /// somebody who picked `alice-signature.png` chose that name on purpose
    /// and typing it again would be busywork — falling back to the same
    /// numbering a drawn signature gets when a file has no useful stem.
    fn signature_name_for(&self, path: &std::path::Path) -> String {
        match path.file_stem().and_then(|s| s.to_str()) {
            Some(stem) if !stem.trim().is_empty() => stem.trim().to_string(),
            _ => format!("Signature {}", self.signatures.entries().len() + 1),
        }
    }

    /// Ask for a picture file, and keep whatever is chosen as a signature.
    fn upload_signature_dialog(&mut self) {
        let dialog = rfd::FileDialog::new()
            .set_title("Upload a signature")
            .add_filter("Picture", &["png", "jpg", "jpeg"]);
        match dialog.pick_file() {
            Some(path) => self.upload_signature(&path),
            None => self.say_info("nothing chosen."),
        }
    }

    /// Decode a picture file and keep it as a signature.
    ///
    /// **Ink, exactly like a drawn one, and said so at the same volume.**
    /// This is a picture — it shows a name, it does not prove one — and
    /// somebody who uploads a signature is exactly as capable of confusing it
    /// with a certificate as somebody who draws one, so the warning belongs
    /// here too, not only on the drawing pad.
    ///
    /// **A phone photo of a signed line, not just a pre-cropped picture.**
    /// Most people do not own a scanner; what they have is a photo of the
    /// page they signed, which carries the whole page — ruled lines, the
    /// table it sat on, whatever bled through from the sheet beneath. This
    /// finds the pen ink in that photo and keeps a tight, whitened crop
    /// around it — see [`pagify_shell::signature_extract`] — falling back
    /// to the picture as given when nothing plausible enough is found,
    /// which also covers a picture that was already just the signature.
    fn upload_signature(&mut self, path: &std::path::Path) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.say_error(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        let not_a_picture = |e: image::ImageError| {
            format!("{} is not a picture this reads (PNG or JPEG): {e}", path.display())
        };
        let reader = match image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format() {
            Ok(reader) => reader,
            Err(e) => {
                self.say_error(not_a_picture(e.into()));
                return;
            }
        };
        let mut decoder = match reader.into_decoder() {
            Ok(decoder) => decoder,
            Err(e) => {
                self.say_error(not_a_picture(e));
                return;
            }
        };
        // Checked from the header, before a single pixel is decoded. The
        // `image` crate caps a decoder's own allocation at 512 MB but not
        // its dimensions, and what follows — `to_rgba8`, orientation, the
        // signature extraction below — each make at least one more
        // full-size copy; the same cap the rest of Pagify holds every
        // raster to. Found by audit.
        // Checked from the header, before a single pixel is decoded. The
        // `image` crate caps a decoder's own allocation at 512 MB but not
        // its dimensions, and what follows — `to_rgba8`, orientation, the
        // signature extraction below — each make at least one more
        // full-size copy; the same cap the rest of Pagify holds every
        // raster to. Found by audit.
        let (declared_width, declared_height) = image::ImageDecoder::dimensions(&decoder);
        if pdf_core::render::bitmap::validate_dimensions(declared_width, declared_height).is_err()
        {
            self.say_error(format!(
                "{} is {declared_width}x{declared_height} — too large a picture to read.",
                path.display()
            ));
            return;
        }
        // A phone photo taken in portrait is very often stored as landscape
        // pixels plus an EXIF tag saying how to rotate it for display —
        // decoding without applying that tag would hand the extraction
        // below a sideways page to search.
        let orientation = image::ImageDecoder::orientation(&mut decoder)
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut decoded = match image::DynamicImage::from_decoder(decoder) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.say_error(not_a_picture(e));
                return;
            }
        };
        decoded.apply_orientation(orientation);
        let photo = decoded.to_rgba8();
        let (photo_width, photo_height) = photo.dimensions();

        let (rgba, width, height) =
            match pagify_shell::signature_extract::extract_signature(&photo, photo_width, photo_height) {
                Some(extracted) => (extracted.rgba, extracted.width, extracted.height),
                None => (photo.into_raw(), photo_width, photo_height),
            };

        let name = self.signature_name_for(path);
        match self.save_uploaded_signature(&name, rgba, width, height) {
            Ok(said) => self.say_info(format!(
                "{said} This is a picture — it shows a name, it does not prove one. \
                 `certify` is what signs with a certificate."
            )),
            Err(e) => self.say_error(e),
        }
    }

    /// Put the drawn or uploaded signature on the line somebody clicked.
    fn place_signature(&mut self, page: usize, at: AppPoint) -> Result<String, String> {
        const WIDTH: f32 = SIGNATURE_WIDTH_PT;
        let Some(signature) = self.signatures.current().cloned() else {
            return Err("no signature has been drawn or uploaded yet — `signature draw` makes \
                        one, `signature upload` adds a picture.".into());
        };

        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        // Placed *and* marked as a signature in one call, so `applysignatures`
        // can tell it from a drawing — or a plain picture — later, including
        // after the document has been closed and reopened.
        if let Some(image) = &signature.image {
            let rect = signature.placed_rect(at.x as f32, at.y as f32, WIDTH);
            doc.session
                .place_image_signature(page, rect, image.rgba.clone(), image.width, image.height, &signature.name)
                .map_err(|e| e.to_string())?;
        } else {
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
            doc.session
                .place_signature(page, strokes, SIGNATURE_INK, 1.6, &signature.name)
                .map_err(|e| e.to_string())?;
        }

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        // **The sentence that keeps the two kinds of signature apart.** Somebody
        // who thinks this is the cryptographic one is worse off than somebody
        // with no signature at all, and this is the line they will read —
        // whichever way this signature was made.
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
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .stamp_mark(
                page,
                mark,
                pdf_core::document::Point { x: at.x as f32, y: at.y as f32 },
                SIZE,
            )
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        Ok(format!("{} on page {}.", mark.describe(), page + 1))
    }

    /// Rule a line while filling a form in.
    fn stamp_line(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .stamp_line(
                page,
                pdf_core::document::Point { x: a.x as f32, y: a.y as f32 },
                pdf_core::document::Point { x: b.x as f32, y: b.y as f32 },
            )
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        Ok(format!("line on page {}.", page + 1))
    }

    /// How close a point comes to a path object's own ink — the minimum
    /// distance to any segment of any sub-path `Session::object_outline`
    /// returned, treating each as an open polyline. Pure and free of
    /// `self`/a session, same reasoning as `nearest_drop`: nothing about
    /// this needs a running document, so it is tested directly instead of
    /// only through a click on one.
    fn distance_to_outline(point: (f32, f32), outline: &[Vec<(f32, f32)>]) -> f32 {
        outline
            .iter()
            .flat_map(|contour| contour.windows(2))
            .map(|pair| Self::distance_to_segment(point, pair[0], pair[1]))
            .fold(f32::INFINITY, f32::min)
    }

    /// The textbook point-to-segment distance: project onto the line through
    /// `a`/`b`, clamp the projection to the segment itself (past either end,
    /// the nearest point on the segment is that endpoint), measure to that.
    fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
        let (abx, aby) = (b.0 - a.0, b.1 - a.1);
        let len_sq = abx * abx + aby * aby;
        if len_sq < 1e-9 {
            return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
        }
        let t = (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len_sq).clamp(0.0, 1.0);
        let (cx, cy) = (a.0 + t * abx, a.1 + t * aby);
        ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
    }

    /// What is under a point, as something that could be picked up.
    ///
    /// Words first, then pictures, then **shapes** — a rule, a box, the panel a
    /// brochure sets its type on. A caption sits *on* a photograph, and
    /// somebody clicking the caption means the caption: the smaller, more
    /// specific thing is what was aimed at, which is the same rule
    /// `pick_text_run` follows among overlapping runs. A shape comes last for
    /// the same reason — a page-wide background is under everything and is
    /// almost never what a click on it means.
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
        let doc = self.tab().doc.as_ref()?;

        // `text_run_rects`, not `text_runs` — a hit-test only needs to know
        // where things are, and `text_runs` pays to extract every run's
        // words to answer that, which on a page of a few hundred runs was
        // most of a second on every single click. See its own doc.
        let words = || {
            doc.session
                .text_run_rects(page)
                .ok()?
                .into_iter()
                .filter(|(_, rect)| holds(rect))
                .min_by(|(_, a), (_, b)| {
                    let area = |r: &pdf_core::document::Rect| {
                        ((r.right - r.left) * (r.bottom - r.top)).abs()
                    };
                    area(a).total_cmp(&area(b))
                })
                .map(|(object, rect)| (object, rect, "the words"))
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
        let shapes = || {
            let area = |d: &pdf_core::document::DrawnObject| {
                ((d.rect.right - d.rect.left) * (d.rect.bottom - d.rect.top)).abs()
            };
            // The bounding box's own area as a last resort — only reached
            // for a path `object_outline` could not read, which in practice
            // should not happen for anything `drawn_objects` calls a
            // `Shape`, but a fallback a future object kind could still need
            // is cheaper than a panic.
            //
            // **Reported from use, on an AutoCAD export**: a bounding box is
            // a poor stand-in for a diagonal line or an arc — the box a long
            // diagonal sweeps out is mostly empty space, and a click
            // anywhere in it used to pick whichever *other* object's box
            // happened to be smallest, usually a nearby outlined-letter path
            // with a tiny box of its own. Ranked by real distance to the
            // object's own ink instead — `(false, distance)` always beats
            // `(true, area)`, so anything with real geometry to measure
            // against is preferred over the fallback, not merely likely to
            // win it by coincidence of scale.
            let rank = |d: &pdf_core::document::DrawnObject| -> (bool, f32) {
                match doc.session.object_outline(page, d.object) {
                    Ok(outline) if !outline.is_empty() => {
                        (false, Self::distance_to_outline((x, y), &outline))
                    }
                    _ => (true, area(d)),
                }
            };
            doc.session
                .drawn_objects(page)
                .ok()?
                .into_iter()
                .filter(|d| {
                    d.depth == 0
                        && d.kind == pdf_core::document::DrawnKind::Shape
                        && holds(&d.rect)
                })
                .min_by(|a, b| {
                    let (a_precise, a_val) = rank(a);
                    let (b_precise, b_val) = rank(b);
                    match (a_precise, b_precise) {
                        (false, true) => std::cmp::Ordering::Less,
                        (true, false) => std::cmp::Ordering::Greater,
                        _ => a_val.total_cmp(&b_val),
                    }
                })
                .map(|d| (d.object, d.rect, "the shape"))
        };

        // **Inside a group, if nothing of the page's own is there.** A brochure
        // draws its panels through a form, and a click on one of those used to
        // find nothing at all — reported from use as the grey layer that could
        // not be selected. What comes back names the *group*, because that is
        // what can be moved; the outline is the thing that was clicked.
        let grouped = || {
            doc.session
                .drawn_objects(page)
                .ok()?
                .into_iter()
                .filter(|d| d.depth > 0 && holds(&d.rect))
                .min_by(|a, b| {
                    let area = |d: &pdf_core::document::DrawnObject| {
                        ((d.rect.right - d.rect.left) * (d.rect.bottom - d.rect.top)).abs()
                    };
                    area(a).total_cmp(&area(b))
                })
                .map(|d| (d.object, d.rect, "the group it is drawn in"))
        };

        if pictures_first {
            pictures().or_else(words).or_else(shapes).or_else(grouped)
        } else {
            words().or_else(pictures).or_else(shapes).or_else(grouped)
        }
    }

    /// Every word-run, picture, and shape on a page whose rectangle overlaps
    /// `rect` at all — what dragging a rectangle over empty page area
    /// collects, for [`Self::group`]. Leaves out anything drawn inside
    /// another object's group (`drawn_objects` depth > 0): a marquee is for
    /// loose page content, and a form's own panels are reached the one-at-
    /// a-time way [`Self::thing_at`]'s own `grouped` closure already reaches
    /// them.
    fn things_in(&self, page: usize, rect: pdf_core::document::Rect) -> Vec<Selected> {
        let norm = |r: &pdf_core::document::Rect| {
            (r.left.min(r.right), r.top.min(r.bottom), r.left.max(r.right), r.top.max(r.bottom))
        };
        let (ml, mt, mr, mb) = norm(&rect);
        // **Fully inside, not merely touched.** A marquee that picked up
        // anything its edge crossed used to sweep in a neighbour that was
        // never meant to be part of the selection — the usual rule in every
        // other drawing or office tool, and the one asked for here.
        let contains = |r: &pdf_core::document::Rect| {
            let (l, t, right, bottom) = norm(r);
            ml <= l && mr >= right && mt <= t && mb >= bottom
        };
        let Some(doc) = self.tab().doc.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        if let Ok(words) = doc.session.text_run_rects(page) {
            out.extend(
                words
                    .into_iter()
                    .filter(|(_, r)| contains(r))
                    .map(|(object, rect)| Selected { page, object, rect, what: "the words" }),
            );
        }
        if let Ok(images) = doc.session.images_on(page) {
            out.extend(images.into_iter().filter(|image| contains(&image.rect)).map(|image| {
                Selected { page, object: image.object, rect: image.rect, what: "the picture" }
            }));
        }
        if let Ok(shapes) = doc.session.drawn_objects(page) {
            out.extend(
                shapes
                    .into_iter()
                    .filter(|d| {
                        d.depth == 0 && d.kind == pdf_core::document::DrawnKind::Shape && contains(&d.rect)
                    })
                    .map(|d| Selected { page, object: d.object, rect: d.rect, what: "the shape" }),
            );
        }
        out
    }

    /// What a marquee drag from `start` to `end` picked up. One thing (or
    /// nothing) behaves exactly like an ordinary click there would have —
    /// [`Self::selected`], with its own handles — so a small rectangle
    /// dragged over a single word is not a worse way to select it than
    /// clicking. More than one becomes [`Self::group`].
    /// `extend`: fold onto the selection already in hand (Shift held) rather
    /// than replace it — added members only, since [`Self::things_in`] only
    /// ever returns what is fully inside the rectangle in the first place.
    fn select_group_in(&mut self, page: usize, start: AppPoint, end: AppPoint, extend: bool) {
        let rect = pdf_core::document::Rect {
            left: start.x.min(end.x) as f32,
            top: start.y.min(end.y) as f32,
            right: start.x.max(end.x) as f32,
            bottom: start.y.max(end.y) as f32,
        };
        let mut found = self.things_in(page, rect);
        if extend {
            let mut members = std::mem::take(&mut self.tab_mut().group);
            if let Some(sel) = self.tab_mut().selected.take() {
                members.push(sel);
            }
            for item in found {
                if !members.iter().any(|s| s.page == item.page && s.object == item.object) {
                    members.push(item);
                }
            }
            found = members;
        } else {
            self.tab_mut().selected = None;
            self.tab_mut().group = Vec::new();
        }
        if found.len() <= 1 {
            if let Some(sel) = found.pop() {
                self.say_info(format!("{} selected.", sel.what));
                self.tab_mut().selected = Some(sel);
            }
            return;
        }
        self.say_info(format!("{} things selected.", found.len()));
        self.tab_mut().group = found;
    }

    /// Shift-click: add whatever is at `at` to the current selection, or —
    /// clicked a second time — drop it again. The usual toggle every other
    /// multi-select gesture uses.
    fn extend_selection_at(&mut self, page: usize, at: AppPoint) {
        let Some(pictures_first) = self.tab_mut().object_tool else { return };
        let Some((object, rect, what)) = self.thing_at(page, at, pictures_first) else { return };
        let mut members = std::mem::take(&mut self.tab_mut().group);
        if let Some(sel) = self.tab_mut().selected.take() {
            members.push(sel);
        }
        match members.iter().position(|s| s.page == page && s.object == object) {
            Some(index) => {
                members.remove(index);
                self.say_info(format!("{what} removed from the selection."));
            }
            None => {
                members.push(Selected { page, object, rect, what });
                self.say_info(format!("{what} added to the selection."));
            }
        }
        if members.len() <= 1 {
            self.tab_mut().selected = members.pop();
        } else {
            self.tab_mut().group = members;
        }
    }

    /// Take the object tool in hand.
    fn take_up_object_tool(&mut self, pictures_first: bool, page: usize) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        self.tab_mut().tool = None;
        self.tab_mut().markup_armed = None;
        self.tab_mut().link_armed = false;
        self.tab_mut().match_properties_armed = false;
        self.tab_mut().match_properties_sample = None;
        self.put_down_page_editors("switched to Edit Object");
        self.tab_mut().object_tool = Some(pictures_first);
        self.tab_mut().selected = None;
        self.tab_mut().grab = None;
        self.tab_mut().group = Vec::new();
        self.tab_mut().marquee = None;
        self.tab_mut().group_grab = None;
        self.tab_mut().signature_selected = None;
        self.tab_mut().signature_grab = None;
        self.tab_mut().placed_image_selected = None;
        self.tab_mut().placed_image_grab = None;
        let _ = page;
        self.say_info(if pictures_first {
            "edit object: click a picture or shape to select it, or words where there is nothing \
             else. Drag to move; drag a handle to resize; Escape puts the tool down."
        } else {
            "move: click words or a picture to select; drag to move, drag a handle to resize."
        });
    }

    /// Select whatever is drawn at a point, or clear the selection.
    ///
    /// A click that landed on text selects the whole **paragraph** it is
    /// part of — see [`Self::page_blocks`] — as a group, the same
    /// [`Self::group`] a marquee drag builds; a paragraph of one line
    /// collapses to a plain [`Self::selected`] exactly as a single-member
    /// marquee already does, keeping today's click-to-a-character behaviour
    /// for the common case of one line on its own.
    fn select_thing_at(&mut self, page: usize, at: AppPoint) -> bool {
        self.select_thing_at_drilling(page, at, true)
    }

    /// The same as [`Self::select_thing_at`], but for a gesture that means
    /// to pick the whole thing up — the start of a drag, or re-finding a
    /// selection right after a move — rather than one that might mean to
    /// look inside it. `drill = false` skips [`Self::split_if_whole_run`]
    /// on an isolated run, leaving it as one object covering every
    /// character rather than the single one under `at`.
    ///
    /// **Reported from use: dragging a line of text moved one letter of it
    /// and left the rest exactly where it was.** `select_thing_at` is also
    /// what a plain click uses to drill into a specific letter for editing
    /// — genuinely wanted there, and kept — but `interact_objects` was
    /// calling that same function to decide what a fresh drag had landed
    /// on, so the drag picked up whichever single character the drill left
    /// selected and moved only that one. A drag means "this, as a whole,
    /// goes where the pointer goes"; only a plain click means "show me
    /// what's exactly here."
    ///
    /// **Never groups into a paragraph.** Edit Object always answers a
    /// click on text at word/letter granularity — that grouping belongs to
    /// Edit Text, where retyping a whole paragraph actually means
    /// something; see [`Self::pick_text_run`].
    fn select_thing_at_drilling(&mut self, page: usize, at: AppPoint, drill: bool) -> bool {
        let Some(pictures_first) = self.tab_mut().object_tool else { return false };
        match self.thing_at(page, at, pictures_first) {
            Some((object, rect, "the words")) => {
                let (object, rect, what) = if drill {
                    self.split_if_whole_run(page, at, object, rect, "the words")
                } else {
                    (object, rect, "the words")
                };
                self.tab_mut().selected = Some(Selected { page, object, rect, what });
                if let Some(index) = self.layer_index_for(page, object, rect) {
                    self.tab_mut().picked_layer = Some(index);
                }
                self.say_info(format!("{what} selected."));
                true
            }
            Some((object, rect, what)) => {
                self.tab_mut().selected = Some(Selected { page, object, rect, what });
                if let Some(index) = self.layer_index_for(page, object, rect) {
                    self.tab_mut().picked_layer = Some(index);
                }
                self.say_info(format!("{what} selected."));
                true
            }
            None => {
                self.tab_mut().selected = None;
                false
            }
        }
    }

    /// A whole run hit by a click becomes one object per character — see
    /// `pdf_core`'s own `split_run_into_characters` — and this re-finds
    /// whichever single character sits at the point that was clicked, since
    /// splitting gives every character in the run a new object number.
    /// Harmless to call on a run an earlier click already split down to one
    /// character: it is a no-op there, so nothing here needs to remember
    /// which runs it has already reached. Left as the whole run, silently,
    /// wherever splitting is declined — a run sharing a line with more text
    /// right after it, say — the same honest-decline the engine already
    /// gives a resize or a delete in that spot.
    fn split_if_whole_run(
        &mut self,
        page: usize,
        at: AppPoint,
        object: usize,
        rect: pdf_core::document::Rect,
        what: &'static str,
    ) -> (usize, pdf_core::document::Rect, &'static str) {
        if what != "the words" {
            return (object, rect, what);
        }
        let Some(doc) = &self.tab_mut().doc else { return (object, rect, what) };
        if doc.session.split_run_into_characters(page, object).is_err() {
            return (object, rect, what);
        }
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        // Not `thing_at`: its few points of slack exist so a click just
        // outside a whole sentence still reaches it, and a dozen characters
        // only a few points wide packed edge to edge — exactly what this
        // just made — turns that same slack into "whichever neighbour
        // happens to be smallest," not the one actually under the pointer.
        // Reported from use as clicking a letter and a different one, or a
        // different run entirely nearby on the page, ending up selected.
        let Some(doc) = &self.tab_mut().doc else { return (object, rect, what) };
        let area = |r: &pdf_core::document::Rect| ((r.right - r.left) * (r.bottom - r.top)).abs();
        let rects = doc.session.text_run_rects(page).ok();
        let hit = rects.as_ref().and_then(|rects| {
            rects
                .iter()
                .filter(|(_, r)| {
                    at.x >= r.left as f64
                        && at.x <= r.right as f64
                        && at.y >= r.top as f64
                        && at.y <= r.bottom as f64
                })
                .min_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)))
                .copied()
        });
        if let Some((object, rect)) = hit {
            return (object, rect, "the letter");
        }
        // **The split already happened — `object` cannot be "left as it
        // was."** The run it named was just replaced by these characters,
        // so returning the pre-split `(object, rect, what)` here would hand
        // back a rect nothing on the page answers to any more, and an
        // object index that now names some *other* character entirely — a
        // move or resize would act on the wrong one, silently. Reported
        // from use: dragging a freshly split run moved one letter of it and
        // left the rest exactly where they were. The exact-bounds check
        // above exists so a tight click cannot miss its own letter onto a
        // packed-in neighbour (see the comment above it); once it has
        // genuinely missed everything, though, correctness matters more
        // than that precision, so this falls back to `thing_at`'s own
        // wider reach rather than lie about nothing having changed.
        let near = HIT_TOLERANCE_PT as f32;
        let loose = rects.and_then(|rects| {
            rects
                .into_iter()
                .filter(|(_, r)| {
                    at.x as f32 >= r.left.min(r.right) - near
                        && at.x as f32 <= r.left.max(r.right) + near
                        && at.y as f32 >= r.top.min(r.bottom) - near
                        && at.y as f32 <= r.top.max(r.bottom) + near
                })
                .min_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)))
        });
        match loose {
            Some((object, rect)) => (object, rect, "the letter"),
            None => (object, rect, what),
        }
    }

    /// The handle under a point on the current selection, if any, allowing
    /// for the handles being drawn at a fixed size on screen.
    fn handle_at(&self, at: AppPoint, view: PageView) -> Option<Handle> {
        let sel = self.tab().selected.as_ref()?;
        // The rotate handle floats above the top edge, so it is asked first: it
        // is the one handle that is not on the rectangle itself.
        let rotate_screen = Self::rotate_handle_screen_pos(&sel.rect, view);
        if (view.to_screen(at) - rotate_screen).length() <= ROTATE_HANDLE_PX + 2.0 {
            return Some(Handle::Rotate);
        }
        Self::handle_near(at, view, &sel.rect)
    }

    /// As [`Self::handle_at`], against an arbitrary rectangle rather than
    /// [`Self::selected`] — what [`Self::group_bounds`]'s own handles use,
    /// since a group has no single object to read a rect from.
    fn handle_near(at: AppPoint, view: PageView, rect: &pdf_core::document::Rect) -> Option<Handle> {
        let reach = (HANDLE_PX / view.scale as f32).max(2.0);
        Handle::ALL.iter().copied().find(|h| {
            let (hx, hy) = h.at(rect);
            (at.x as f32 - hx).abs() <= reach && (at.y as f32 - hy).abs() <= reach
        })
    }

    /// The union of every [`Self::group`] member's rectangle on `page` — the
    /// box its own resize handles sit on. `None` with fewer than one member
    /// there, same as no selection at all.
    fn group_bounds(&self, page: usize) -> Option<pdf_core::document::Rect> {
        let mut members = self.tab().group.iter().filter(|m| m.page == page).map(|m| m.rect);
        let mut bounds = members.next()?;
        for r in members {
            bounds.left = bounds.left.min(r.left);
            bounds.top = bounds.top.min(r.top);
            bounds.right = bounds.right.max(r.right);
            bounds.bottom = bounds.bottom.max(r.bottom);
        }
        Some(bounds)
    }

    /// How near, in screen pixels, a side or the middle of a thing being moved has
    /// to be to a reference line to be pulled onto it.
    const GUIDE_SNAP_PX: f32 = 6.0;
    /// How near a reference line is shown at all.
    const GUIDE_NEAR_PX: f32 = 40.0;

    /// The rectangles the thing being moved can line up with: everything else
    /// the page draws that is a thing of its own — not the page-sized
    /// background, not a speck, not what is being moved.
    fn guide_targets(&mut self, page: usize, exclude: &[usize]) -> Vec<pdf_core::document::Rect> {
        let size = self.tab().doc.as_ref().and_then(|d| d.strip.size_of(page));
        let layers = self.layers_on(page);
        // A page of tens of thousands of drawn pieces is a map, not something
        // to align to; and scanning it every frame of a drag would show.
        if layers.len() > 20_000 {
            return Vec::new();
        }
        layers
            .iter()
            .filter(|o| o.depth <= 2 && !exclude.contains(&o.object))
            .map(|o| o.rect)
            .filter(|r| {
                let (w, h) = ((r.right - r.left).abs(), (r.bottom - r.top).abs());
                // The page's own background covers all of it and lines up with
                // nothing but the page, whose edges are lines already.
                let background = size.is_some_and(|(pw, ph)| w >= pw * 0.95 && h >= ph * 0.95);
                w >= 1.0 && h >= 1.0 && !background
            })
            .collect()
    }

    /// Where `moving` would sit if it were pulled onto the nearest reference
    /// line, and the lines to show for it. `snap` is off while Alt is held: the
    /// lines still show, and nothing pulls.
    fn align_for(
        &mut self,
        page: usize,
        moving: pdf_core::document::Rect,
        exclude: &[usize],
        scale: f32,
        snap: bool,
    ) -> ((f32, f32), pagify_shell::guides::Guides) {
        let size = self.tab().doc.as_ref().and_then(|d| d.strip.size_of(page)).unwrap_or((612.0, 792.0));
        let targets = self.guide_targets(page, exclude);
        let scale = scale.max(0.01);
        pagify_shell::guides::align(
            moving,
            &targets,
            size,
            if snap { Self::GUIDE_SNAP_PX / scale } else { 0.0 },
            Self::GUIDE_NEAR_PX / scale,
        )
    }

    /// **While a thing is dragged by its body, pull it onto a reference line.**
    ///
    /// Asked for with a screenshot: lines through the edges and the middles of
    /// the other things on the page, and the thing landing exactly on one.
    /// Applied to the drag itself, every frame, from the pointer — so the ghost
    /// drawn while it moves and the place it is dropped are the same place, and
    /// letting go away from a line is not "sticky".
    fn snap_the_move(&mut self, page: usize, scale: f32, snap: bool) {
        if let (Some(grab), Some(sel)) = (self.tab().grab.clone(), self.tab().selected.clone()) {
            if grab.handle.is_none() && sel.page == page {
                let moving = Self::shifted(sel.rect, grab.by);
                let (nudge, _) = self.align_for(page, moving, &[sel.object], scale, snap);
                if let Some(g) = self.tab_mut().grab.as_mut() {
                    g.by = (g.by.0 + nudge.0, g.by.1 + nudge.1);
                }
            }
        }
        if let Some(grab) = self.tab().group_grab.clone() {
            if grab.handle.is_none() {
                if let Some(bounds) = self.group_bounds(page) {
                    let members: Vec<usize> =
                        self.tab().group.iter().filter(|m| m.page == page).map(|m| m.object).collect();
                    let moving = Self::shifted(bounds, grab.by);
                    let (nudge, _) = self.align_for(page, moving, &members, scale, snap);
                    if let Some(g) = self.tab_mut().group_grab.as_mut() {
                        g.by = (g.by.0 + nudge.0, g.by.1 + nudge.1);
                    }
                }
            }
        }
    }

    fn shifted(r: pdf_core::document::Rect, by: (f32, f32)) -> pdf_core::document::Rect {
        pdf_core::document::Rect { left: r.left + by.0, top: r.top + by.1, right: r.right + by.0, bottom: r.bottom + by.1 }
    }

    /// The union, in app space, of every shape the markup layer's own
    /// selection holds on `page` — the box its rotate handle sits on. `None`
    /// with nothing selected there, same as [`Self::group_bounds`].
    fn markup_selection_bounds(&self, page: usize) -> Option<pdf_core::document::Rect> {
        let layer = self.tab().markup.existing(page)?;
        let space = layer.space();
        let mut corners = layer.selection().iter().flat_map(|&index| {
            let (min, max) = layer.objects().get(index)?.bbox();
            Some([space.from_kernel(min), space.from_kernel(max)])
        }).flatten();
        let first = corners.next()?;
        let mut bounds = pdf_core::document::Rect {
            left: first.x as f32,
            right: first.x as f32,
            top: first.y as f32,
            bottom: first.y as f32,
        };
        for p in corners {
            bounds.left = bounds.left.min(p.x as f32);
            bounds.right = bounds.right.max(p.x as f32);
            bounds.top = bounds.top.min(p.y as f32);
            bounds.bottom = bounds.bottom.max(p.y as f32);
        }
        Some(bounds)
    }


    /// The least a click's own press-to-release wobble has to move the
    /// pointer, in screen pixels, before it counts as a deliberate drag
    /// rather than an unsteady hand — see [`PagifyApp::finish_grab`].
    const MIN_DRAG_PX: f32 = 3.0;

    /// Apply what a drag asked for, once, and re-find the selection where it
    /// now is.
    ///
    /// `scale` — screen pixels per page point, [`PageView::scale`] — is what
    /// [`Self::MIN_DRAG_PX`] is measured against, not `grab.by` itself:
    /// `grab.by` is in page points, and the same few points of press-to-
    /// release wobble are a fraction of a pixel at one zoom and several
    /// pixels at another. Reported from use as a click moving whatever it
    /// selected — invisible on a whole sentence, glaring on the single
    /// letter a click can now pick out of one.
    fn finish_grab(&mut self, sel: Selected, grab: Grab, scale: f32) {
        let (dx, dy) = grab.by;
        // `handle: None` here is exactly the "aimed at a corner, landed on
        // the body" failure mode `Self::object_hover_handle` exists to
        // prevent — logged before the outcome message below, which only
        // ever says "moved"/"resized" and cannot by itself say which one a
        // reporter actually meant to happen.
        self.session_log.record(
            "drag",
            &format!("{} handle={:?} by=({dx:.1},{dy:.1}) rect={:?}", sel.what, grab.handle, sel.rect),
        );
        let told = match grab.handle {
            None => {
                if (dx * scale).hypot(dy * scale) < Self::MIN_DRAG_PX {
                    return;
                }
                // Not `move_thing`: that re-finds the object via `thing_at`,
                // which for a run of words means a full-page `text_runs()`
                // scan — and `sel.object` already names exactly which one
                // this drag picked up, back when it was selected.
                self.move_object_by(sel.page, sel.object, sel.what, (dx, dy))
            }
            // **Turned about its own middle.** Clockwise as seen, which is what the
            // drag swept; the same turn the other way is the undo.
            Some(Handle::Rotate) => {
                let degrees = Self::object_turn(&sel.rect, &grab, self.tab().rotate_snap);
                if degrees.abs() < 0.5 {
                    return;
                }
                let pivot = pdf_core::document::Point {
                    x: (sel.rect.left + sel.rect.right) / 2.0,
                    y: (sel.rect.top + sel.rect.bottom) / 2.0,
                };
                self.rotate_thing(sel.page, sel.object, pivot, degrees)
            }
            Some(handle) => {
                let (sx, sy) = handle.scale(&sel.rect, (dx, dy));
                if (sx - 1.0).abs() < 0.005 && (sy - 1.0).abs() < 0.005 {
                    return;
                }
                let (ax, ay) = handle.anchor(&sel.rect);
                self.scale_thing(sel.page, sel.object, pdf_core::document::Point { x: ax, y: ay }, sx, sy)
            }
        };
        match told {
            Ok(said) => self.say_info(said),
            Err(e) => self.say_error(e),
        }

        // Where it is now: the same kind of thing, at the rectangle the drag
        // implied. Found again because the page was rewritten and the object
        // numbers may have moved.
        let wanted = match grab.handle {
            None => pdf_core::document::Rect {
                left: sel.rect.left + dx,
                top: sel.rect.top + dy,
                right: sel.rect.right + dx,
                bottom: sel.rect.bottom + dy,
            },
            // Turned about its middle: the middle stays where it is, which is all
            // that the finding-again below looks at.
            Some(Handle::Rotate) => sel.rect,
            Some(handle) => {
                let (sx, sy) = handle.scale(&sel.rect, (dx, dy));
                let (ax, ay) = handle.anchor(&sel.rect);
                pdf_core::document::Rect {
                    left: ax + (sel.rect.left - ax) * sx,
                    top: ay + (sel.rect.top - ay) * sy,
                    right: ax + (sel.rect.right - ax) * sx,
                    bottom: ay + (sel.rect.bottom - ay) * sy,
                }
            }
        };
        let middle = AppPoint {
            x: ((wanted.left + wanted.right) / 2.0) as f64,
            y: ((wanted.top + wanted.bottom) / 2.0) as f64,
        };
        self.forget_layers();
        // Not a drill: `sel.what` already says what kind of thing this drag
        // moved, and re-running the letter-drill here on whatever now sits
        // at the new centre could pick a different granularity than the one
        // actually dragged.
        if !self.select_thing_at_drilling(sel.page, middle, false) {
            self.tab_mut().selected = None;
        }
    }

    /// Move or resize every member of [`Self::group`] together. A move is a
    /// plain translation, so (unlike the resize branch) each member's new
    /// rectangle is exactly the old one shifted, nothing to re-find or
    /// correct for. A resize scales every member by the same `sx`/`sy`
    /// about one shared anchor — [`Handle::scale`]/[`Handle::anchor`]
    /// applied to [`Self::group_bounds`] instead of one object's own rect —
    /// so the group keeps its shape exactly as a single object's own resize
    /// would, just with several objects moving together instead of one.
    /// `scale` is [`PageView::scale`] — see [`Self::finish_grab`] for why
    /// the drag-was-negligible guard is measured in screen pixels rather
    /// than page points.
    fn finish_group_grab(&mut self, grab: Grab, scale: f32) {
        let (dx, dy) = grab.by;
        match grab.handle {
            None => {
                if (dx * scale).hypot(dy * scale) < Self::MIN_DRAG_PX {
                    return;
                }
                let members = std::mem::take(&mut self.tab_mut().group);
                let total = members.len();
                let page = members.first().map(|m| m.page);
                let mut moved = 0;
                let mut last_err = None;
                for member in &members {
                    match self.move_object_by(member.page, member.object, member.what, (dx, dy)) {
                        Ok(_) => moved += 1,
                        Err(e) => last_err = Some(e),
                    }
                }
                self.tab_mut().group = members
                    .into_iter()
                    .map(|mut m| {
                        m.rect.left += dx;
                        m.rect.right += dx;
                        m.rect.top += dy;
                        m.rect.bottom += dy;
                        m
                    })
                    .collect();
                self.forget_layers();
                match (last_err, page) {
                    (Some(e), _) => self.say_error(format!("moved {moved} of {total} things; {e}")),
                    (None, Some(page)) => self.say_info(format!(
                        "moved {total} things by {dx:.0} across and {dy:.0} down on page {}.",
                        page + 1
                    )),
                    (None, None) => {}
                }
            }
            Some(handle) => {
                let Some(page) = self.tab_mut().group.first().map(|m| m.page) else { return };
                let Some(bounds) = self.group_bounds(page) else { return };
                let (sx, sy) = handle.scale(&bounds, (dx, dy));
                if (sx - 1.0).abs() < 0.005 && (sy - 1.0).abs() < 0.005 {
                    return;
                }
                let (ax, ay) = handle.anchor(&bounds);
                let anchor = pdf_core::document::Point { x: ax, y: ay };
                let members = std::mem::take(&mut self.tab_mut().group);
                let total = members.len();
                let mut done = 0;
                let mut last_err = None;
                for member in &members {
                    match self.scale_thing(member.page, member.object, anchor, sx, sy) {
                        Ok(_) => done += 1,
                        Err(e) => last_err = Some(e),
                    }
                }
                self.tab_mut().group = members
                    .into_iter()
                    .map(|mut m| {
                        m.rect = pdf_core::document::Rect {
                            left: ax + (m.rect.left - ax) * sx,
                            top: ay + (m.rect.top - ay) * sy,
                            right: ax + (m.rect.right - ax) * sx,
                            bottom: ay + (m.rect.bottom - ay) * sy,
                        };
                        m
                    })
                    .collect();
                self.forget_layers();
                match last_err {
                    Some(e) => self.say_error(format!("resized {done} of {total} things; {e}")),
                    None => self.say_info(format!(
                        "resized {total} things to {:.0}% across and {:.0}% down on page {}.",
                        sx * 100.0,
                        sy * 100.0,
                        page + 1
                    )),
                }
            }
        }
    }

    /// Delete every member of [`Self::group`], highest object index first.
    /// A page's later objects shift down to fill a deleted one's slot, so
    /// deleting low-to-high would have the second removal already reading a
    /// stale index — the same reason a picture's own object number can move
    /// after any edit, just now happening mid-batch instead of between one
    /// drag and the next.
    fn delete_group(&mut self) {
        let mut members = std::mem::take(&mut self.tab_mut().group);
        members.sort_by(|a, b| b.object.cmp(&a.object));
        let total = members.len();
        let page = members.first().map(|m| m.page);
        let mut removed = 0;
        let mut last_err = None;
        for member in &members {
            let result = match &self.tab_mut().doc {
                Some(doc) => doc
                    .session
                    .execute(pdf_core::command::Command::RemoveObject {
                        page_index: member.page,
                        object: member.object,
                    })
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                None => Err("nothing open.".into()),
            };
            match result {
                Ok(()) => removed += 1,
                Err(e) => last_err = Some(e),
            }
        }
        if removed > 0 {
            if let Some(doc) = &mut self.tab_mut().doc {
                doc.rendered_is_stale();
            }
        }
        self.forget_layers();
        match (last_err, page) {
            (Some(e), _) if removed == 0 => self.say_error(e),
            (Some(e), _) => self.say_error(format!("removed {removed} of {total} things; {e}")),
            (None, Some(page)) => self.say_info(format!("{removed} things removed from page {}.", page + 1)),
            (None, None) => {}
        }
    }

    /// Turn something about a point, clockwise as seen.
    fn rotate_thing(
        &mut self,
        page: usize,
        object: usize,
        pivot: pdf_core::document::Point,
        degrees: f32,
    ) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .execute(pdf_core::command::Command::RotateObject { page_index: page, object, pivot, degrees })
            .map_err(|e| explain(&e))?;
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        // Said the way the label says it: counter-clockwise is positive.
        Ok(format!("turned {:.0}\u{b0} on page {} — `undo` turns it back.", -degrees, page + 1))
    }

    /// Resize something about a point.
    fn scale_thing(
        &mut self,
        page: usize,
        object: usize,
        anchor: pdf_core::document::Point,
        sx: f32,
        sy: f32,
    ) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .execute(pdf_core::command::Command::ScaleObject { page_index: page, object, anchor, sx, sy })
            .map_err(|e| e.to_string())?;
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        Ok(format!(
            "resized to {:.0}% across and {:.0}% down on page {}.",
            sx * 100.0,
            sy * 100.0,
            page + 1
        ))
    }

    /// The placed-but-unapplied picture signature under `at` on `page`, if
    /// any — the smallest one, where more than one overlaps, the same
    /// tie-break [`Self::thing_at`] uses.
    fn signature_at(&self, page: usize, at: AppPoint) -> Option<(usize, pdf_core::document::Rect, f32)> {
        let near = HIT_TOLERANCE_PT as f32;
        let (x, y) = (at.x as f32, at.y as f32);
        let doc = self.tab().doc.as_ref()?;
        doc.session
            .image_signature_marks(page)
            .ok()?
            .into_iter()
            .filter(|m| {
                x >= m.rect.left.min(m.rect.right) - near
                    && x <= m.rect.left.max(m.rect.right) + near
                    && y >= m.rect.top.min(m.rect.bottom) - near
                    && y <= m.rect.top.max(m.rect.bottom) + near
            })
            .min_by(|a, b| {
                let area = |r: &pdf_core::document::Rect| ((r.right - r.left) * (r.bottom - r.top)).abs();
                area(&a.rect).total_cmp(&area(&b.rect))
            })
            .map(|m| (m.index, m.rect, m.rotation))
    }

    /// The same as [`Self::signature_at`], for a plain placed picture.
    fn placed_image_at(&self, page: usize, at: AppPoint) -> Option<(usize, pdf_core::document::Rect, f32)> {
        let near = HIT_TOLERANCE_PT as f32;
        let (x, y) = (at.x as f32, at.y as f32);
        let doc = self.tab().doc.as_ref()?;
        doc.session
            .placed_image_marks(page)
            .ok()?
            .into_iter()
            .filter(|m| {
                x >= m.rect.left.min(m.rect.right) - near
                    && x <= m.rect.left.max(m.rect.right) + near
                    && y >= m.rect.top.min(m.rect.bottom) - near
                    && y <= m.rect.top.max(m.rect.bottom) + near
            })
            .min_by(|a, b| {
                let area = |r: &pdf_core::document::Rect| ((r.right - r.left) * (r.bottom - r.top)).abs();
                area(&a.rect).total_cmp(&area(&b.rect))
            })
            .map(|m| (m.index, m.rect, m.rotation))
    }

    /// Whether `at` falls within `rect` — no tolerance, unlike hit-testing
    /// for a pick: this is "is the pointer over the already-selected body",
    /// asked every frame for the cursor and the drag-start decision alike.
    fn point_in_rect(at: AppPoint, rect: &pdf_core::document::Rect) -> bool {
        at.x >= rect.left.min(rect.right) as f64
            && at.x <= rect.left.max(rect.right) as f64
            && at.y >= rect.top.min(rect.bottom) as f64
            && at.y <= rect.top.max(rect.bottom) as f64
    }

    /// The handle under a point on the current signature selection, if any
    /// — the same reach-allowing-for-screen-size math as [`Self::handle_at`],
    /// kept separate because it reads [`Self::signature_selected`] rather
    /// than [`Self::selected`]. Also checks the rotate handle, which
    /// `Handle::ALL` does not include (see [`Handle::Rotate`]).
    fn signature_handle_at(&self, at: AppPoint, view: PageView) -> Option<Handle> {
        let sel = self.tab().signature_selected.as_ref()?;
        let rotate_screen = Self::rotate_handle_screen_pos(&sel.rect, view);
        if (view.to_screen(at) - rotate_screen).length() <= ROTATE_HANDLE_PX + 2.0 {
            return Some(Handle::Rotate);
        }
        let reach = (HANDLE_PX / view.scale as f32).max(2.0);
        Handle::ALL.iter().copied().find(|h| {
            let (hx, hy) = h.at(&sel.rect);
            (at.x as f32 - hx).abs() <= reach && (at.y as f32 - hy).abs() <= reach
        })
    }

    /// The same as [`Self::signature_handle_at`], for [`Self::placed_image_selected`].
    fn placed_image_handle_at(&self, at: AppPoint, view: PageView) -> Option<Handle> {
        let sel = self.tab().placed_image_selected.as_ref()?;
        let rotate_screen = Self::rotate_handle_screen_pos(&sel.rect, view);
        if (view.to_screen(at) - rotate_screen).length() <= ROTATE_HANDLE_PX + 2.0 {
            return Some(Handle::Rotate);
        }
        let reach = (HANDLE_PX / view.scale as f32).max(2.0);
        Handle::ALL.iter().copied().find(|h| {
            let (hx, hy) = h.at(&sel.rect);
            (at.x as f32 - hx).abs() <= reach && (at.y as f32 - hy).abs() <= reach
        })
    }

    /// Where the rotate handle is drawn and hit-tested — a fixed distance
    /// above the rect's top-centre **in screen space**, whatever the zoom,
    /// the same way every app that has one places it. Unlike the eight
    /// resize handles (proportional to the rect, so they sit exactly on
    /// its own corners and edges at any zoom), this one has no page-space
    /// equivalent: "24 pixels" is not a page distance.
    fn rotate_handle_screen_pos(rect: &pdf_core::document::Rect, view: PageView) -> egui::Pos2 {
        let top_centre = view.to_screen(AppPoint::new(((rect.left + rect.right) / 2.0) as f64, rect.top as f64));
        top_centre - egui::vec2(0.0, ROTATE_HANDLE_OFFSET_PX)
    }

    /// The angle a rotate-drag means: `base_rotation` (the signature's own
    /// rotation when the drag started) plus how far the pointer has swept
    /// clockwise around `rect`'s centre between `from` and `from + by`.
    ///
    /// **Why raw `atan2`, with no sign flip.** [`image_placement_matrix`]
    /// (`pdf_core`) needs one, because PDF space is y-up and a clockwise
    /// screen turn is counterclockwise there. App space — what `rect`,
    /// `from` and `by` are all already in — is y-down, the same sense a
    /// clock face is normally drawn in, so `atan2(dy, dx)` already increases
    /// clockwise on its own; negating it here would turn the picture the
    /// wrong way when dragged.
    fn angle_from_drag(rect: &pdf_core::document::Rect, base_rotation: f32, from: AppPoint, by: (f32, f32)) -> f32 {
        let centre = ((rect.left + rect.right) as f64 / 2.0, (rect.top + rect.bottom) as f64 / 2.0);
        let start = (from.y - centre.1).atan2(from.x - centre.0);
        let now = ((from.y + by.1 as f64) - centre.1).atan2((from.x + by.0 as f64) - centre.0);
        base_rotation + (now - start).to_degrees() as f32
    }


    /// Apply what a drag on a signature asked for, once — the same shape as
    /// [`Self::finish_grab`]. A rotate-handle drag writes through
    /// `Session::rotate_image_signature`; body and resize-handle drags
    /// write through `Session::set_image_signature_rect`, as before — never
    /// `move_thing`/`scale_thing`, since a signature's rect (and now angle)
    /// is the whole of what moving, resizing or turning it means; there is
    /// no content-stream object underneath to transform.
    fn finish_signature_grab(&mut self, sel: SignatureSelected, grab: Grab) {
        self.session_log.record(
            "drag",
            &format!("signature handle={:?} by={:?} rect={:?}", grab.handle, grab.by, sel.rect),
        );
        if grab.handle == Some(Handle::Rotate) {
            let wanted = Self::angle_from_drag(&sel.rect, sel.rotation, grab.from, grab.by);
            if (wanted - sel.rotation).abs() < 0.5 {
                return;
            }
            let Some(doc) = &self.tab_mut().doc else { return };
            match doc.session.rotate_image_signature(sel.page, sel.index, wanted) {
                Ok(()) => {
                    if let Some(doc) = &mut self.tab_mut().doc {
                        doc.rendered_is_stale();
                    }
                    self.tab_mut().signature_selected =
                        Some(SignatureSelected { rotation: wanted, ..sel });
                }
                Err(e) => {
                    self.say_error(e.to_string());
                    self.tab_mut().signature_selected = Some(sel);
                }
            }
            return;
        }

        let (dx, dy) = grab.by;
        let wanted = match grab.handle {
            None => {
                if dx.abs() < 0.5 && dy.abs() < 0.5 {
                    return;
                }
                pdf_core::document::Rect {
                    left: sel.rect.left + dx,
                    top: sel.rect.top + dy,
                    right: sel.rect.right + dx,
                    bottom: sel.rect.bottom + dy,
                }
            }
            Some(handle) => {
                let (sx, sy) = handle.scale(&sel.rect, (dx, dy));
                if (sx - 1.0).abs() < 0.005 && (sy - 1.0).abs() < 0.005 {
                    return;
                }
                let (ax, ay) = handle.anchor(&sel.rect);
                pdf_core::document::Rect {
                    left: ax + (sel.rect.left - ax) * sx,
                    top: ay + (sel.rect.top - ay) * sy,
                    right: ax + (sel.rect.right - ax) * sx,
                    bottom: ay + (sel.rect.bottom - ay) * sy,
                }
            }
        };
        let Some(doc) = &self.tab_mut().doc else { return };
        match doc.session.set_image_signature_rect(sel.page, sel.index, wanted) {
            Ok(()) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.tab_mut().signature_selected = Some(SignatureSelected { rect: wanted, ..sel });
            }
            Err(e) => {
                self.say_error(e.to_string());
                // Left where it was rather than guessed at — the annotation
                // itself did not move if the call failed partway, and a
                // selection rect that disagreed with it would make the next
                // drag start from the wrong place.
                self.tab_mut().signature_selected = Some(sel);
            }
        }
    }


    /// The same as [`Self::finish_signature_grab`], for [`Self::placed_image_selected`].
    fn finish_placed_image_grab(&mut self, sel: PlacedImageSelected, grab: Grab) {
        self.session_log.record(
            "drag",
            &format!("placed image handle={:?} by={:?} rect={:?}", grab.handle, grab.by, sel.rect),
        );
        if grab.handle == Some(Handle::Rotate) {
            let wanted = Self::angle_from_drag(&sel.rect, sel.rotation, grab.from, grab.by);
            if (wanted - sel.rotation).abs() < 0.5 {
                return;
            }
            let Some(doc) = &self.tab_mut().doc else { return };
            match doc.session.rotate_image_signature(sel.page, sel.index, wanted) {
                Ok(()) => {
                    if let Some(doc) = &mut self.tab_mut().doc {
                        doc.rendered_is_stale();
                    }
                    self.tab_mut().placed_image_selected =
                        Some(PlacedImageSelected { rotation: wanted, ..sel });
                }
                Err(e) => {
                    self.say_error(e.to_string());
                    self.tab_mut().placed_image_selected = Some(sel);
                }
            }
            return;
        }

        let (dx, dy) = grab.by;
        let wanted = match grab.handle {
            None => {
                if dx.abs() < 0.5 && dy.abs() < 0.5 {
                    return;
                }
                pdf_core::document::Rect {
                    left: sel.rect.left + dx,
                    top: sel.rect.top + dy,
                    right: sel.rect.right + dx,
                    bottom: sel.rect.bottom + dy,
                }
            }
            Some(handle) => {
                let (sx, sy) = handle.scale(&sel.rect, (dx, dy));
                if (sx - 1.0).abs() < 0.005 && (sy - 1.0).abs() < 0.005 {
                    return;
                }
                let (ax, ay) = handle.anchor(&sel.rect);
                pdf_core::document::Rect {
                    left: ax + (sel.rect.left - ax) * sx,
                    top: ay + (sel.rect.top - ay) * sy,
                    right: ax + (sel.rect.right - ax) * sx,
                    bottom: ay + (sel.rect.bottom - ay) * sy,
                }
            }
        };
        let Some(doc) = &self.tab_mut().doc else { return };
        match doc.session.set_image_signature_rect(sel.page, sel.index, wanted) {
            Ok(()) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.tab_mut().placed_image_selected = Some(PlacedImageSelected { rect: wanted, ..sel });
            }
            Err(e) => {
                self.say_error(e.to_string());
                self.tab_mut().placed_image_selected = Some(sel);
            }
        }
    }

    /// Apply a drag on the markup layer's own selection — the same shape as
    /// [`Self::finish_grab`], but committed through `tools::move_selection`
    /// rather than `Command::MoveObject`: a drawn shape is neither page
    /// content nor an annotation, and every selected object moves together
    /// by the same delta, the same way the typed `move` command already
    /// works, this being the drag that reaches it instead of two picks.
    fn finish_markup_grab(&mut self, page: usize, grab: Grab) {
        let (dx, dy) = grab.by;
        if dx.hypot(dy) < Self::MIN_DRAG_PX {
            return;
        }
        self.session_log.record("drag", &format!("markup by=({dx:.1},{dy:.1})"));
        let height = view_height(self, page);
        let layer = self.tab_mut().markup.page(page, height);
        layer.begin("move");
        // App space counts downwards, kernel space upwards — see
        // `PageSpace::to_kernel`. A pure reflection, not a rotation or a
        // scale, so the x half of a *delta* carries over unchanged and only
        // y flips; there is no point to convert through, only a direction.
        let moved = tools::move_selection(layer, cad_kernel::Vec2::new(dx as f64, -dy as f64));
        layer.end();
        self.say_info(format!("{moved} moved."));
    }

    /// Apply a rotate-handle drag on the markup layer's own selection — the
    /// angle comes from [`Self::angle_from_drag`], the same maths the
    /// signature and picture rotate handles already use, but committed
    /// through `tools::rotate_selection` since a drawn shape has no stored
    /// `rotation` field of its own to overwrite: it is real geometry, turned
    /// about the selection's own centre.
    ///
    /// **The sign flip `finish_markup_grab` does not need.** `angle_from_
    /// drag` reports clockwise-positive in app space, which is the sense a
    /// drag should visibly turn the shape in; `Geom::rotated`'s `angle` is
    /// the standard counter-clockwise-positive convention of a *y-up* frame,
    /// which — unlike a plain delta — does not survive `PageSpace`'s y-flip
    /// unchanged. Verified with a real drag rather than derived on paper:
    /// see `dragging_the_rotate_handle_turns_a_drawn_shape`.
    fn finish_markup_rotate(&mut self, page: usize, grab: Grab, bounds: pdf_core::document::Rect) {
        let degrees = Self::angle_from_drag(&bounds, 0.0, grab.from, grab.by);
        if degrees.abs() < 1.0 {
            return;
        }
        self.session_log.record("drag", &format!("markup rotate by={degrees:.1}deg"));
        let height = view_height(self, page);
        let layer = self.tab_mut().markup.page(page, height);
        let space = layer.space();
        let centre = AppPoint::new(
            ((bounds.left + bounds.right) / 2.0) as f64,
            ((bounds.top + bounds.bottom) / 2.0) as f64,
        );
        let pivot = space.to_kernel(centre);
        layer.begin("rotate");
        let n = tools::rotate_selection(layer, pivot, -(degrees as f64).to_radians());
        layer.end();
        self.say_info(format!("{n} rotated."));
    }

    /// Make the selected thing more or less see-through.
    fn set_opacity_of(&mut self, page: usize, object: usize, opacity: f32) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session.set_opacity(page, object, opacity).map_err(|e| e.to_string())?;
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        Ok(format!("opacity {:.0}% on page {}.", opacity * 100.0, page + 1))
    }


    /// The turn a rotate drag means, in degrees **clockwise as seen**, snapped to
    /// the nearest 15 when `snap` (Shift held), and kept in `-180..=180`.
    fn object_turn(rect: &pdf_core::document::Rect, grab: &Grab, snap: bool) -> f32 {
        let mut degrees = Self::angle_from_drag(rect, 0.0, grab.from, grab.by);
        // Wrapped, so sweeping past straight down reads -170 and not 190.
        degrees = (degrees + 180.0).rem_euclid(360.0) - 180.0;
        if snap {
            degrees = (degrees / 15.0).round() * 15.0;
        }
        degrees
    }

    /// The ring with a turning arrow in it that marks every rotate handle.
    fn draw_rotate_icon(painter: &egui::Painter, centre: egui::Pos2) {
        let r = ROTATE_HANDLE_PX;
        painter.circle_filled(centre, r, egui::Color32::WHITE);
        painter.circle_stroke(centre, r, egui::Stroke::new(1.0, theme::violet()));
        // Three quarters of a circle, and a head on its end.
        let arc = egui::Stroke::new(1.3, theme::violet());
        let inner = r * 0.55;
        let points: Vec<egui::Pos2> = (0..=18)
            .map(|i| {
                let a = (-60.0f32 + 20.0 * i as f32).to_radians();
                centre + egui::vec2(a.cos(), a.sin()) * inner
            })
            .collect();
        painter.add(egui::Shape::line(points.clone(), arc));
        if let (Some(&end), Some(&before)) = (points.last(), points.get(points.len() - 2)) {
            let dir = (end - before).normalized();
            let side = egui::vec2(-dir.y, dir.x);
            painter.add(egui::Shape::convex_polygon(
                vec![end + dir * 2.4, end - dir * 0.6 + side * 2.2, end - dir * 0.6 - side * 2.2],
                theme::violet(),
                egui::Stroke::NONE,
            ));
        }
    }

    /// The label beside the pointer while turning: the angle, as the other
    /// programs write it — **counter-clockwise is positive**, so a turn clockwise
    /// reads `-33°`.
    fn draw_angle_label(painter: &egui::Painter, near: egui::Pos2, clockwise_degrees: f32) {
        let shown = -clockwise_degrees;
        let text = format!("{:.0}\u{b0}", if shown == 0.0 { 0.0 } else { shown });
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), egui::Color32::BLACK);
        let size = galley.size() + egui::vec2(10.0, 6.0);
        let rect = egui::Rect::from_min_size(near + egui::vec2(16.0, -size.y - 6.0), size);
        painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(255, 250, 205));
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_gray(90)), egui::StrokeKind::Inside);
        painter.galley(rect.min + egui::vec2(5.0, 3.0), galley, egui::Color32::BLACK);
    }

    /// The marquee rectangle while it is being dragged out; every member of
    /// [`Self::group`] once one exists, moving or resizing together while a
    /// drag is live; and, when nothing is being dragged, the group's own
    /// bounding box and its resize handles — the same drawing
    /// [`Self::draw_object_selection`] gives a single object, just built
    /// from [`Self::group_bounds`] instead of one object's own rect.
    fn draw_group_selection(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let to_screen = |r: &pdf_core::document::Rect| {
            egui::Rect::from_min_max(
                view.to_screen(AppPoint::new(r.left as f64, r.top as f64)),
                view.to_screen(AppPoint::new(r.right as f64, r.bottom as f64)),
            )
        };
        let painter = ui.painter();

        if let Some((start, current)) = self.tab_mut().marquee {
            let rect = egui::Rect::from_two_pos(view.to_screen(start), view.to_screen(current));
            painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::violet().gamma_multiply(0.08));
            painter.rect_stroke(
                rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, theme::violet()),
                egui::StrokeKind::Outside,
            );
        }

        let Some(bounds) = self.group_bounds(page) else { return };

        // Where a rectangle is headed, given the drag in progress — a
        // translation for a body drag, a scale about the group's own anchor
        // corner for a handle, same maths `Self::finish_group_grab` commits
        // with on release.
        let going = |r: &pdf_core::document::Rect| -> pdf_core::document::Rect {
            let Some(grab) = &self.tab().group_grab else { return *r };
            let (dx, dy) = grab.by;
            match grab.handle {
                None => pdf_core::document::Rect {
                    left: r.left + dx,
                    top: r.top + dy,
                    right: r.right + dx,
                    bottom: r.bottom + dy,
                },
                Some(handle) => {
                    let (sx, sy) = handle.scale(&bounds, (dx, dy));
                    let (ax, ay) = handle.anchor(&bounds);
                    pdf_core::document::Rect {
                        left: ax + (r.left - ax) * sx,
                        top: ay + (r.top - ay) * sy,
                        right: ax + (r.right - ax) * sx,
                        bottom: ay + (r.bottom - ay) * sy,
                    }
                }
            }
        };

        for member in self.tab().group.iter().filter(|m| m.page == page) {
            let outline = to_screen(&going(&member.rect));
            if self.tab().group_grab.is_some() {
                painter.rect_filled(outline, egui::CornerRadius::ZERO, theme::violet().gamma_multiply(0.10));
            }
            painter.rect_stroke(
                outline,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.5, theme::violet_bright()),
                egui::StrokeKind::Outside,
            );
        }

        // The handles, at a fixed size on screen — hidden while a drag is
        // live, same as a single object's own (see `draw_object_selection`).
        if self.tab_mut().group_grab.is_some() {
            return;
        }
        painter.rect_stroke(
            to_screen(&bounds),
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, theme::violet()),
            egui::StrokeKind::Outside,
        );
        for handle in Handle::ALL {
            let (hx, hy) = handle.at(&bounds);
            let centre = view.to_screen(AppPoint::new(hx as f64, hy as f64));
            let square = egui::Rect::from_center_size(centre, egui::Vec2::splat(HANDLE_PX * 2.0));
            painter.rect_filled(square, egui::CornerRadius::ZERO, egui::Color32::WHITE);
            painter.rect_stroke(
                square,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, theme::violet()),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// The signature selection's outline and handles — the same drawing as
    /// [`Self::draw_object_selection`], reading [`Self::signature_selected`]
    /// and [`Self::signature_grab`] instead. Kept as its own function rather
    /// than a parameter on that one: the two selections are independent by
    /// design (see [`SignatureSelected`]), and a shared draw call would be
    /// the one place that quietly assumed otherwise.
    fn draw_signature_selection(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let Some(sel) = self.tab_mut().signature_selected.clone().filter(|s| s.page == page) else { return };
        let to_screen = |r: &pdf_core::document::Rect| {
            egui::Rect::from_min_max(
                view.to_screen(AppPoint::new(r.left as f64, r.top as f64)),
                view.to_screen(AppPoint::new(r.right as f64, r.bottom as f64)),
            )
        };
        let painter = ui.painter();
        let outline = to_screen(&sel.rect);

        painter.rect_stroke(
            outline,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.5, theme::violet()),
            egui::StrokeKind::Outside,
        );

        if let Some(grab) = &self.tab_mut().signature_grab {
            // Turning does not move `sel.rect` at all — see `SignatureSelected`
            // and `Annotation::Image`'s own doc for why rotation is a
            // separate transform, not a change to the rect move and resize
            // share. The outline above already shows the (unrotated)
            // footprint correctly; what a turn-in-progress needs is the
            // angle itself, shown as a line from the centre out to the
            // pointer and the number of degrees it now stands at.
            if grab.handle == Some(Handle::Rotate) {
                let degrees = Self::angle_from_drag(&sel.rect, sel.rotation, grab.from, grab.by);
                let centre = to_screen(&sel.rect).center();
                let pointer = view.to_screen(AppPoint::new(
                    grab.from.x + grab.by.0 as f64,
                    grab.from.y + grab.by.1 as f64,
                ));
                painter.line_segment([centre, pointer], egui::Stroke::new(1.5, theme::violet_bright()));
                painter.text(
                    pointer + egui::vec2(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}°", degrees.rem_euclid(360.0)),
                    egui::FontId::monospace(13.0),
                    theme::violet_bright(),
                );
                return;
            }
            let (dx, dy) = grab.by;
            let going = match grab.handle {
                None => pdf_core::document::Rect {
                    left: sel.rect.left + dx,
                    top: sel.rect.top + dy,
                    right: sel.rect.right + dx,
                    bottom: sel.rect.bottom + dy,
                },
                Some(handle) => {
                    let (sx, sy) = handle.scale(&sel.rect, (dx, dy));
                    let (ax, ay) = handle.anchor(&sel.rect);
                    pdf_core::document::Rect {
                        left: ax + (sel.rect.left - ax) * sx,
                        top: ay + (sel.rect.top - ay) * sy,
                        right: ax + (sel.rect.right - ax) * sx,
                        bottom: ay + (sel.rect.bottom - ay) * sy,
                    }
                }
            };
            let ghost = to_screen(&going);
            painter.rect_filled(ghost, egui::CornerRadius::ZERO, theme::violet().gamma_multiply(0.10));
            painter.rect_stroke(
                ghost,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.5, theme::violet_bright()),
                egui::StrokeKind::Outside,
            );
            return;
        }

        for handle in Handle::ALL {
            let (hx, hy) = handle.at(&sel.rect);
            let centre = view.to_screen(AppPoint::new(hx as f64, hy as f64));
            let square = egui::Rect::from_center_size(centre, egui::Vec2::splat(HANDLE_PX * 2.0));
            painter.rect_filled(square, egui::CornerRadius::ZERO, egui::Color32::WHITE);
            painter.rect_stroke(
                square,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, theme::violet()),
                egui::StrokeKind::Inside,
            );
        }

        // The rotate handle: a ring above the top edge, joined to it by a
        // short stem — the standard shape for "drag to turn" wherever it
        // shows up, and clearly not one of the eight resize handles.
        let rotate_screen = Self::rotate_handle_screen_pos(&sel.rect, view);
        let stem_from = view.to_screen(AppPoint::new(
            ((sel.rect.left + sel.rect.right) / 2.0) as f64,
            sel.rect.top as f64,
        ));
        painter.line_segment([stem_from, rotate_screen], egui::Stroke::new(1.0, theme::violet()));
        Self::draw_rotate_icon(painter, rotate_screen);
    }

    /// The same as [`Self::draw_signature_selection`], for
    /// [`Self::placed_image_selected`] and [`Self::placed_image_grab`].
    fn draw_placed_image_selection(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let Some(sel) = self.tab_mut().placed_image_selected.clone().filter(|s| s.page == page) else { return };
        let to_screen = |r: &pdf_core::document::Rect| {
            egui::Rect::from_min_max(
                view.to_screen(AppPoint::new(r.left as f64, r.top as f64)),
                view.to_screen(AppPoint::new(r.right as f64, r.bottom as f64)),
            )
        };
        let painter = ui.painter();
        let outline = to_screen(&sel.rect);

        painter.rect_stroke(
            outline,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.5, theme::violet()),
            egui::StrokeKind::Outside,
        );

        if let Some(grab) = &self.tab_mut().placed_image_grab {
            if grab.handle == Some(Handle::Rotate) {
                let degrees = Self::angle_from_drag(&sel.rect, sel.rotation, grab.from, grab.by);
                let centre = to_screen(&sel.rect).center();
                let pointer = view.to_screen(AppPoint::new(
                    grab.from.x + grab.by.0 as f64,
                    grab.from.y + grab.by.1 as f64,
                ));
                painter.line_segment([centre, pointer], egui::Stroke::new(1.5, theme::violet_bright()));
                painter.text(
                    pointer + egui::vec2(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}°", degrees.rem_euclid(360.0)),
                    egui::FontId::monospace(13.0),
                    theme::violet_bright(),
                );
                return;
            }
            let (dx, dy) = grab.by;
            let going = match grab.handle {
                None => pdf_core::document::Rect {
                    left: sel.rect.left + dx,
                    top: sel.rect.top + dy,
                    right: sel.rect.right + dx,
                    bottom: sel.rect.bottom + dy,
                },
                Some(handle) => {
                    let (sx, sy) = handle.scale(&sel.rect, (dx, dy));
                    let (ax, ay) = handle.anchor(&sel.rect);
                    pdf_core::document::Rect {
                        left: ax + (sel.rect.left - ax) * sx,
                        top: ay + (sel.rect.top - ay) * sy,
                        right: ax + (sel.rect.right - ax) * sx,
                        bottom: ay + (sel.rect.bottom - ay) * sy,
                    }
                }
            };
            let ghost = to_screen(&going);
            painter.rect_filled(ghost, egui::CornerRadius::ZERO, theme::violet().gamma_multiply(0.10));
            painter.rect_stroke(
                ghost,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.5, theme::violet_bright()),
                egui::StrokeKind::Outside,
            );
            return;
        }

        for handle in Handle::ALL {
            let (hx, hy) = handle.at(&sel.rect);
            let centre = view.to_screen(AppPoint::new(hx as f64, hy as f64));
            let square = egui::Rect::from_center_size(centre, egui::Vec2::splat(HANDLE_PX * 2.0));
            painter.rect_filled(square, egui::CornerRadius::ZERO, egui::Color32::WHITE);
            painter.rect_stroke(
                square,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, theme::violet()),
                egui::StrokeKind::Inside,
            );
        }

        let rotate_screen = Self::rotate_handle_screen_pos(&sel.rect, view);
        let stem_from = view.to_screen(AppPoint::new(
            ((sel.rect.left + sel.rect.right) / 2.0) as f64,
            sel.rect.top as f64,
        ));
        painter.line_segment([stem_from, rotate_screen], egui::Stroke::new(1.0, theme::violet()));
        Self::draw_rotate_icon(painter, rotate_screen);
    }

    /// The markup layer's own selection needs no outline of its own — a
    /// selected shape is already the violet-highlighted stroke
    /// `overlay::draw_layer` paints it with — so this draws only what that
    /// does not: the rotate handle, and, mid-drag, the angle it is turning
    /// to. The same ring-on-a-stem [`Self::draw_signature_selection`] uses,
    /// since a drawn shape gained a rotate handle for the same reason a
    /// signature already has one — see [`Self::finish_markup_rotate`].
    fn draw_markup_selection(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let Some(bounds) = self.markup_selection_bounds(page) else { return };
        let painter = ui.painter();

        if let Some(grab) = &self.tab_mut().markup_grab {
            // A move in progress needs no handle in the way of watching the
            // shape itself go; a rotate in progress shows the angle instead
            // of the resting ring.
            if grab.handle == Some(Handle::Rotate) {
                let degrees = Self::angle_from_drag(&bounds, 0.0, grab.from, grab.by);
                let centre = view.to_screen(AppPoint::new(
                    ((bounds.left + bounds.right) / 2.0) as f64,
                    ((bounds.top + bounds.bottom) / 2.0) as f64,
                ));
                let pointer = view.to_screen(AppPoint::new(
                    grab.from.x + grab.by.0 as f64,
                    grab.from.y + grab.by.1 as f64,
                ));
                painter.line_segment([centre, pointer], egui::Stroke::new(1.5, theme::violet_bright()));
                painter.text(
                    pointer + egui::vec2(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{:.0}°", degrees.rem_euclid(360.0)),
                    egui::FontId::monospace(13.0),
                    theme::violet_bright(),
                );
            }
            return;
        }

        let rotate_screen = Self::rotate_handle_screen_pos(&bounds, view);
        let stem_from = view.to_screen(AppPoint::new(
            ((bounds.left + bounds.right) / 2.0) as f64,
            bounds.top as f64,
        ));
        painter.line_segment([stem_from, rotate_screen], egui::Stroke::new(1.0, theme::violet()));
        Self::draw_rotate_icon(painter, rotate_screen);
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
        let by = ((to.x - from.x) as f32, (to.y - from.y) as f32);
        self.move_object_by(page, object, what, by)
    }

    /// The move itself, given the object already — no hit-test. `move_thing`
    /// above is `thing_at` plus this; a caller that already knows the object
    /// (the object tool's own drag, which has `Selected::object` before the
    /// drag ever starts) must not be made to pay for rediscovering it.
    ///
    /// **Why this needed splitting out at all.** `thing_at`'s text branch
    /// calls `text_runs()` — the full-page scan `text_run_at` exists to let
    /// single-object callers skip — so every drag-to-move went through it a
    /// second time on top of whatever `move_object` itself cost, even after
    /// that was already fixed. Measured on a real page: ~800ms of a
    /// still-slow move was this call alone, invisible in a benchmark of
    /// `move_object` because that benchmark never called `thing_at` at all.
    fn move_object_by(
        &mut self,
        page: usize,
        object: usize,
        what: &'static str,
        by: (f32, f32),
    ) -> Result<String, String> {
        if by.0.abs() < 0.1 && by.1.abs() < 0.1 {
            return Err("move: that is where it already is.".into());
        }
        let point = pdf_core::document::Point { x: by.0, y: by.1 };

        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .execute(pdf_core::command::Command::MoveObject { page_index: page, object, by: point })
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        Ok(format!(
            "moved {what} by {:.0} across and {:.0} down on page {}.",
            by.0,
            by.1,
            page + 1
        ))
    }

    /// Draw a box around something while filling a form in.
    fn stamp_box(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        let Some(area) = area_between(a, b) else {
            return Err("rectangle: that area has no size.".into());
        };
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session.stamp_box(page, area).map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.tab_mut().doc {
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
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .whiteout(page, area, pdf_core::document::Color { r: 255, g: 255, b: 255, a: 255 })
            .map_err(|e| e.to_string())?;

        if let Some(doc) = &mut self.tab_mut().doc {
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
        self.tab().doc.as_ref().is_some_and(|d| d.session.is_secured())
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
        self.tab().doc.as_ref().is_some_and(|d| d.session.is_dirty().unwrap_or(false))
    }

    /// Whether closing, or opening something else, would throw work away.
    ///
    /// One question with one answer, asked everywhere that closes something.
    /// Six places asked their own version of it, and every one of them knew
    /// about marks while five knew nothing about edits.
    fn would_lose_work(&self) -> bool {
        self.unsaved().is_some() || self.unsaved_password() || self.unsaved_edits()
    }


    /// Rebuild this page's reading order and show the result.
    ///
    /// Asked for rather than applied. The disorder score is reported alongside,
    /// because on real documents it is a poor guide: a CAD drawing scores 0.637
    /// and reads perfectly, while the deliberately scrambled fixture scores
    /// 0.267 and does not.
    fn reflow(&mut self) {
        use pdf_core::document::layout;

        let page = self.tab_mut().page;
        let Some(doc) = &self.tab_mut().doc else {
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

    /// Look for `needle` on every page and keep what was found, **without
    /// moving to any of it** — `find_at` is left as it was, for a caller that
    /// is searching again from where it already is. `false`, once said, when
    /// there is no document to search.
    fn search(&mut self, needle: &str) -> bool {
        let Some(doc) = self.tab().doc.as_ref() else {
            self.say_error("nothing open.");
            return false;
        };
        let session = doc.session.clone();
        let page_count = doc.page_count;

        self.tab_mut().find_hits.clear();
        self.tab_mut().find_needle = needle.to_string();

        for page in 0..page_count {
            let Ok(chars) = session.characters(page) else { continue };
            for hit in chars.find(needle) {
                self.tab_mut().find_hits.push((page, hit));
            }
        }
        true
    }

    /// Search every page, and go to the first match.
    fn find(&mut self, needle: &str) {
        if !self.search(needle) {
            return;
        }
        self.tab_mut().find_at = 0;

        if self.tab().find_hits.is_empty() {
            self.say_info(format!("`{needle}` — no matches."));
            return;
        }

        let pages = {
            let mut seen: Vec<usize> = self.tab().find_hits.iter().map(|(p, _)| *p).collect();
            seen.dedup();
            seen.len()
        };
        let count = self.tab().find_hits.len();
        self.say_info(format!(
            "{count} match{} on {pages} page{}. `findnext` steps through them.",
            if count == 1 { "" } else { "es" },
            if pages == 1 { "" } else { "s" },
        ));
        self.go_to_hit(0);
    }

    fn find_step(&mut self, forward: bool) {
        if self.tab_mut().find_hits.is_empty() {
            self.say_error("nothing to step through — `find <text>` first.");
            return;
        }
        let count = self.tab_mut().find_hits.len();
        let next = if forward {
            (self.tab_mut().find_at + 1) % count
        } else {
            (self.tab_mut().find_at + count - 1) % count
        };
        self.go_to_hit(next);
    }

    fn go_to_hit(&mut self, index: usize) {
        let Some((page, range)) = self.tab_mut().find_hits.get(index).cloned() else { return };
        self.tab_mut().find_at = index;

        // **Where the word is, not only which page it is on.** Every way of
        // finding something ends here, so this is the one place that makes
        // sure it can be seen — reported from use as the page coming up with
        // the highlight somewhere below the bottom of the window.
        //
        // A turned view has no rectangle to give: the page-space box does not
        // say where the word is on screen there, so it is left to the page.
        let spot = self
            .characters(page)
            .and_then(|chars| chars.line_rects(range.clone()).first().copied())
            .filter(|_| matches!(self.tab().rotation, Rotation::None));
        if spot.is_some() || page != self.tab_mut().page {
            // Through `go_to` rather than by hand, so the scroll position, the
            // page-size cache and the raster cache all move together — they are
            // three things that must not disagree about which page is showing.
            //
            // Also when the page is already the current one: `go_to`'s request
            // for the page's top is what the reveal settles for when the word
            // is in the first screenful, and costs nothing when it is not.
            self.go_to(PageTarget::Number(page + 1));
        }
        self.tab_mut().reveal = spot.map(|rect| Reveal { page, rect });
        // The match is also the selection, so ⌘C copies what was found.
        self.tab_mut().text_selection = Some(range);
        let current_page = self.tab().page;
        self.tab_mut().selection_page = current_page;
        let total = self.tab().find_hits.len();
        self.say_info(format!("match {} of {total}", index + 1));
    }

    /// Replace every occurrence of `needle` across the whole document,
    /// matched the same case- and shape-folded way `find` already does — so
    /// Search & Replace never disagrees with Find about what a search term
    /// means.
    ///
    /// **A run at a time, not a character at a time.** Only a run whose own
    /// text holds a match end to end can be rewritten in place, the same
    /// safe swap a single run's own retyping already uses — see
    /// `Command::SetTextRun`. A word a producer drew across two runs (a
    /// font change or a hyphen break mid-word) has no one text-showing
    /// operation to rewrite, so it is left alone rather than guessed at;
    /// `pick_text_run`'s own paragraph-joining logic exists for reading a
    /// run split that way, not for editing it back together.
    ///
    /// Sending `TextStyle::default()` — nothing asked for — is what keeps
    /// every replaced word in its own size, colour, font and position: see
    /// `apply_one_edit`'s own doc for why an empty style is what sends an
    /// edit down the byte-safe path that touches only the words themselves.
    fn replace_all(&mut self, needle: &str, replacement: &str) -> Result<String, String> {
        if needle.trim().is_empty() {
            return Err("nothing to search for.".into());
        }
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };

        let mut replaced = 0usize;
        let mut runs_touched = 0usize;
        for page in 0..doc.page_count {
            let Ok(runs) = doc.session.text_runs(page) else { continue };
            for run in &runs {
                let hits = pdf_core::document::search::SearchIndex::new(&run.text).find(needle);
                if hits.is_empty() {
                    continue;
                }
                let mut new_text = String::with_capacity(run.text.len());
                let mut last = 0;
                for hit in &hits {
                    new_text.push_str(&run.text[last..hit.start]);
                    new_text.push_str(replacement);
                    last = hit.end;
                }
                new_text.push_str(&run.text[last..]);

                if doc
                    .session
                    .execute(pdf_core::command::Command::SetTextRun {
                        page_index: page,
                        object: run.object,
                        text: new_text,
                        style: pdf_core::document::TextStyle::default(),
                    })
                    .is_ok()
                {
                    replaced += hits.len();
                    runs_touched += 1;
                }
            }
        }

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();

        if replaced == 0 {
            return Ok(format!("\"{needle}\" was not found."));
        }
        Ok(format!(
            "replaced {replaced} occurrence{} of \"{needle}\" across {runs_touched} run{}. \
             `undo` puts them back, one at a time.",
            if replaced == 1 { "" } else { "s" },
            if runs_touched == 1 { "" } else { "s" },
        ))
    }

    /// Replace just the current match — `self.tab_mut().find_hits[self.tab_mut().find_at]` —
    /// and step to whatever is now the next one, leaving every other match
    /// exactly as it was.
    ///
    /// **Searches again rather than tracking a position by hand.** Once one
    /// match is rewritten, every later match on that page has shifted by
    /// however many characters the replacement's length differs from the
    /// word searched for — re-running the search is what makes "the next
    /// match" a fresh, correct answer instead of a stale offset. It goes on
    /// from where the replacement ends, not from the first match again: that
    /// is the spot just written, and (when the replacement still contains the
    /// word, "the" to "other") the first match found would be that spot itself.
    ///
    /// **A match is shown before it is replaced.** Reported from use: with a
    /// different word typed in since the last search, Replace changed the first
    /// match without it ever having been on screen. Now that press only shows
    /// it, and the next one replaces what is on screen.
    ///
    /// Finds the run the same way a click would: the smallest run whose own
    /// rectangle contains the match's own — not `pick_text_run`, which
    /// would grow a single word into the whole paragraph around it. Only
    /// the one occurrence the match actually is gets replaced, not every
    /// occurrence the run might otherwise hold, and not the *first* one it
    /// holds either: in "the cat and the dog" the second "the" is the match
    /// the reader stepped to, and it is the one that changes.
    fn replace_current(&mut self, needle: &str, replacement: &str) -> Result<String, String> {
        if self.tab().find_hits.is_empty() || self.tab().find_needle != needle {
            self.find(needle);
            return match self.tab().find_hits.len() {
                0 => Err(format!("\"{needle}\" was not found.")),
                n => Ok(format!(
                    "match 1 of {n} — press Replace again to replace it, or Find Next to leave it."
                )),
            };
        }
        let find_at = self.tab().find_at;
        let Some((page, range)) = self.tab().find_hits.get(find_at).cloned() else {
            return Err(format!("\"{needle}\" was not found."));
        };
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            return Err("nothing open.".into());
        };

        let chars = session.characters(page).map_err(|e| e.to_string())?;
        let centre = |r: &pagify_shell::reader::Rect| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let spot = chars
            .line_rects(range.clone())
            .into_iter()
            .next()
            .ok_or("that match could not be placed on the page.")?;
        let point = centre(&spot);

        let runs = session.text_runs(page).map_err(|e| e.to_string())?;
        let area = |r: &pdf_core::document::Rect| {
            (r.right - r.left).abs() * (r.bottom - r.top).abs()
        };
        let run_at = |point: (f32, f32)| {
            runs.iter()
                .filter(|r| {
                    let (left, right) = (r.rect.left.min(r.rect.right), r.rect.left.max(r.rect.right));
                    let (top, bottom) = (r.rect.top.min(r.rect.bottom), r.rect.top.max(r.rect.bottom));
                    point.0 >= left && point.0 <= right && point.1 >= top && point.1 <= bottom
                })
                .min_by(|a, b| area(&a.rect).total_cmp(&area(&b.rect)))
        };
        let run = run_at(point).cloned().ok_or("that match is not inside any run this can edit.")?;

        // Which of the run's own matches this is: the matches come in reading
        // order, so it is the one after as many as the earlier hits that sit
        // in this same run — found the same way, by where they are.
        let earlier = self.tab().find_hits[..find_at]
            .iter()
            .filter(|(p, r)| {
                *p == page
                    && chars
                        .line_rects(r.clone())
                        .first()
                        .and_then(|s| run_at(centre(s)))
                        .is_some_and(|other| other.object == run.object)
            })
            .count();
        let here = pdf_core::document::search::SearchIndex::new(&run.text)
            .find(needle)
            .get(earlier)
            .cloned()
            .ok_or("that match could not be found in its own run.")?;
        let mut new_text = run.text.clone();
        new_text.replace_range(here, replacement);
        // Where the replacement ends, as a position among the page's characters:
        // everything before the replaced word has not moved.
        let resume = (page, range.start + replacement.chars().count());

        session
            .execute(pdf_core::command::Command::SetTextRun {
                page_index: page,
                object: run.object,
                text: new_text,
                style: pdf_core::document::TextStyle::default(),
            })
            .map_err(|e| explain(&e))?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();

        self.search(needle);
        let left = self.tab().find_hits.len();
        if left == 0 {
            return Ok(format!("replaced the last \"{needle}\" — none left."));
        }
        // The first match from there on, or the first of all when there is
        // none: Replace runs round the document the way Find Next does.
        let next = self.tab().find_hits.iter().position(|(p, r)| (*p, r.start) >= resume).unwrap_or(0);
        self.go_to_hit(next);
        Ok(format!("replaced 1 occurrence of \"{needle}\" — {left} left. `undo` puts it back."))
    }


    /// Scan every page for a word the bundled dictionary does not know —
    /// the PDF's own original text and anything typed in through the app
    /// alike, since both are ordinary page content by the time this reads
    /// them (`write_text_at`/`write_styled_line_at` write real text
    /// objects, not an overlay — see `pdfium_doc.rs`'s own `TEXT_MARK_NAME`
    /// doc for why that tag exists only to find them again, not to make
    /// them a separate kind of thing).
    ///
    /// Which words are skipped, and which pages are, is
    /// `spelling::misspelled_in_page`'s to say: a single letter, an acronym
    /// or a model code, and anything the English list cannot judge (another
    /// script, an accent, a digit) are left alone. A page mostly in a script
    /// it cannot judge is returned in the second list, not as clean.
    ///
    /// Runs on the scan's own thread — see [`SpellScan`]. `on_page` is told how
    /// many pages are done, and `None` comes back when `stop` was set.
    fn scan_spelling(
        session: &Session,
        page_count: usize,
        stop: &std::sync::atomic::AtomicBool,
        mut on_page: impl FnMut(usize),
    ) -> Option<(Vec<Misspelling>, Vec<usize>, bool)> {
        let (mut found, mut skipped, mut chinese) = (Vec::new(), Vec::new(), false);
        for page in 0..page_count {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }
            if let Ok(runs) = session.text_runs(page) {
                let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
                chinese |= spelling::has_chinese(&texts);
                match spelling::misspelled_in_page(&texts) {
                    None => skipped.push(page),
                    Some(words) => found.extend(words.into_iter().map(|(run, word)| Misspelling {
                        page,
                        object: runs[run].object,
                        word: word.to_string(),
                    })),
                }
            }
            on_page(page + 1);
        }
        Some((found, skipped, chinese))
    }

    /// Open the panel and start the scan at once — "when user clicks it, it
    /// should check the spelling," not open first and wait for a second
    /// press. The panel opens showing how far the scan has got; the first word
    /// replaces that when the scan finishes (see [`Self::collect_spell_scan`]).
    fn open_spell_check(&mut self) {
        let Some((session, page_count)) =
            self.tab().doc.as_ref().map(|d| (d.session.clone(), d.page_count))
        else {
            self.say_error("nothing open.");
            return;
        };
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, done) = std::sync::mpsc::channel();
        let worker_stop = stop.clone();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let result = Self::scan_spelling(&session, page_count, &worker_stop, |n| {
                let _ = progress.send(ScanMessage::Progress(n));
            });
            if let Some((found, skipped, chinese)) = result {
                let _ = tx.send(ScanMessage::Finished { found, skipped, chinese });
            }
        });
        // Replacing a scan still running drops it, which stops it.
        self.tab_mut().spell_scan = Some(SpellScan { done, stop, pages: page_count });
        self.tab_mut().spelling =
            Some(SpellCheck { scanning: Some((0, page_count)), ..SpellCheck::default() });
    }

    /// Take what the scan has reported since the last frame: its progress, and
    /// at the end the words, which turn the panel from "checking" into the
    /// first word to review.
    fn collect_spell_scan(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.tab().spell_scan else { return };
        let (mut checked, mut finished, mut ended) = (None, None, false);
        loop {
            match scan.done.try_recv() {
                Ok(ScanMessage::Progress(n)) => checked = Some(n),
                Ok(ScanMessage::Finished { found, skipped, chinese }) => {
                    finished = Some((found, skipped, chinese));
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Keep the frames coming, or the count stands still until
                    // the mouse moves.
                    ctx.request_repaint_after(std::time::Duration::from_millis(60));
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    ended = true;
                    break;
                }
            }
        }
        let pages = scan.pages;

        if let Some(n) = checked {
            if let Some(panel) = self.tab_mut().spelling.as_mut() {
                panel.scanning = Some((n, pages));
            }
        }
        if ended {
            // The thread went away without an answer and nobody asked it to.
            self.tab_mut().spell_scan = None;
            self.tab_mut().spelling = None;
            self.say_error("the spelling check stopped before it finished.");
            return;
        }
        let Some((found, skipped_pages, chinese)) = finished else { return };

        self.tab_mut().spell_scan = None;
        let total_found = found.len();
        let mut panel =
            SpellCheck { found, total_found, skipped_pages, chinese, ..SpellCheck::default() };
        panel.reset_replacement();
        if total_found > 0 {
            self.say_info(format!(
                "{total_found} word{} to review.",
                if total_found == 1 { "" } else { "s" }
            ));
        } else if panel.skipped_pages.is_empty() {
            self.say_info("no misspelled words found.");
        } else {
            // The panel says which pages; this must not read as a clean bill
            // of health for pages nothing looked at.
            self.say_info("no misspelled words found in what could be checked.");
        }
        self.tab_mut().spelling = Some(panel);
    }

    /// Block until the scan has reported.
    ///
    /// Tests only; the application never waits.
    #[cfg(test)]
    fn wait_for_spell_scan(&mut self) {
        let ctx = egui::Context::default();
        for _ in 0..1200 {
            if self.tab().spell_scan.is_none() {
                return;
            }
            self.collect_spell_scan(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the spelling scan did not finish within twelve seconds");
    }

    /// Write `replacement` over one occurrence of `word` in the run named by
    /// `page`/`object`.
    ///
    /// **Finds the word in the run's own *current* text, not by a byte
    /// offset kept from when the page was scanned.** An earlier fix to the
    /// same run has already changed its length by however many characters
    /// the two words differ by — the same reasoning `replace_current`
    /// documents for why it searches again rather than tracking a
    /// position. `TextStyle::default()` — nothing asked for — is what
    /// keeps the fix in the word's own size, colour, font and position.
    fn apply_spelling_change(
        &mut self,
        page: usize,
        object: usize,
        word: &str,
        replacement: &str,
    ) -> Result<(), String> {
        if replacement.trim().is_empty() {
            return Err("nothing to change it to.".into());
        }
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        let runs = doc.session.text_runs(page).map_err(|e| e.to_string())?;
        let run = runs
            .iter()
            .find(|r| r.object == object)
            .ok_or("that run is no longer on the page.")?;
        let at = run
            .text
            .find(word)
            .ok_or("that word could not be found in its own run any more.")?;
        let mut new_text = run.text.clone();
        new_text.replace_range(at..at + word.len(), replacement);

        doc.session
            .execute(pdf_core::command::Command::SetTextRun {
                page_index: page,
                object,
                text: new_text,
                style: pdf_core::document::TextStyle::default(),
            })
            .map_err(|e| explain(&e))?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        Ok(())
    }


    /// Add a bookmark for the current page and show the panel it now
    /// appears in.
    ///
    /// **Named from the current text selection when there is one** — the
    /// same instinct as typing a caption from the words already picked,
    /// rather than "Page 7" telling nobody what is actually there. Falls
    /// back to the page number when there is nothing selected, or the
    /// selection is empty once trimmed.
    fn add_bookmark_here(&mut self) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let page = self.tab_mut().page;
        let title = self.tab_mut()
            .text_selection
            .clone()
            .filter(|_| self.tab_mut().selection_page == page)
            .and_then(|range| self.characters(page).map(|c| c.text_of(range)))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Page {}", page + 1));

        let Some(doc) = &self.tab_mut().doc else { return };
        match doc.session.execute(pdf_core::command::Command::AddBookmark {
            title: title.clone(),
            page_index: page,
        }) {
            Ok(_) => {
                self.say_info(format!("bookmarked \"{title}\"."));
                let entries = self.sync_bookmarks();
                self.tab_mut().bookmark_panel = Some(BookmarkPanel { entries });
            }
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Read the document's own bookmarks once, updating both
    /// `bookmarked_pages` (the page-corner icon's own cache) and, only when
    /// it is already open, the panel's own list.
    ///
    /// **Never opens the panel itself** — called after undo, redo and
    /// opening a document, none of which should pop a panel open that
    /// nobody asked to see. `add_bookmark_here` opens it explicitly, using
    /// the same entries this returns.
    fn sync_bookmarks(&mut self) -> Vec<(String, usize)> {
        let entries = self.tab_mut().doc.as_ref().and_then(|d| d.session.bookmarks().ok()).unwrap_or_default();
        self.tab_mut().bookmarked_pages = entries.iter().map(|(_, page)| *page).collect();
        if self.tab_mut().bookmark_panel.is_some() {
            self.tab_mut().bookmark_panel = Some(BookmarkPanel { entries: entries.clone() });
        }
        entries
    }

    /// Open or close the Bookmarks panel without adding one — the rail's
    /// own icon-row button next to the thumbnail view, mirroring the
    /// mockup's own rail header (`code.html:296-322`). Unlike
    /// `add_bookmark_here`, this never writes to the document.
    fn toggle_bookmark_panel(&mut self) {
        if self.tab_mut().bookmark_panel.is_some() {
            self.tab_mut().bookmark_panel = None;
        } else {
            let entries = self.sync_bookmarks();
            self.tab_mut().bookmark_panel = Some(BookmarkPanel { entries });
        }
    }


    /// Web Links — a text selection already made is linked at once; nothing
    /// selected arms the tool and waits for one, the same shape
    /// `mark_selection` uses for the highlighter and its siblings.
    fn begin_web_link(&mut self) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        if self.tab_mut().text_selection.is_some() {
            self.open_link_prompt_from_selection();
        } else {
            self.put_down_page_editors("armed a web link");
            self.tab_mut().markup_armed = None;
            self.tab_mut().match_properties_armed = false;
            self.tab_mut().match_properties_sample = None;
            self.tab_mut().link_armed = true;
            self.say_info(
                "web link — drag across the text to link it (typed text works too, once it \
                 is on the page). Escape puts it down.",
            );
        }
    }

    /// Turn the current text selection into a pending link, waiting for the
    /// address to send it to.
    fn open_link_prompt_from_selection(&mut self) {
        let Some(range) = self.tab_mut().text_selection.clone() else { return };
        let page = self.tab_mut().selection_page;
        let rects = self.selection_rects(page, range);
        if rects.is_empty() {
            self.say_error("that selection has nothing to link.");
            return;
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().pending_link = Some(PendingLink { page, rects, url: String::new() });
    }

    /// A text selection's line rects, in the engine's own coordinate type —
    /// the conversion a web link and a manual paragraph join both start
    /// from, written once. Empty wherever the reader found nothing to box.
    fn selection_rects(&mut self, page: usize, range: std::ops::Range<usize>) -> Vec<pdf_core::document::Rect> {
        let Some(rects) = self.characters(page).map(|c| c.line_rects(range)) else {
            return Vec::new();
        };
        // The reader's rect and the engine's are the same numbers in the
        // same space, and two distinct types — see `mark_selection`'s own
        // identical conversion.
        rects
            .into_iter()
            .map(|r| pdf_core::document::Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            })
            .collect()
    }

    /// A run's own embedded font, registered under its `/BaseFont` name and
    /// ready to hand to `TextStyle::face` — the same registration
    /// `writing_faces` does for a font read from a file, sourced from a
    /// font already living in the page instead.
    ///
    /// **Registered in both of the app's two separate font pools, not
    /// just one.** `pdf_core::text::register` is what `write_styled_line_at`
    /// reads for laying new text out on screen; `add_typing_font` is the
    /// entirely separate list `SetTextRun`'s own `embed_typing_font` checks
    /// when a *requested* face is asked to redraw an existing run — the
    /// exact path `match_font_to_first_selected` sends every style change
    /// through. Registering only the first left every requested face
    /// refused with "is not one of the fonts available to write with",
    /// regardless of whether the font could actually spell the words —
    /// the font was simply never in the one list this call path checks.
    /// See the upload-a-font-file call site (search `add_typing_font`) for
    /// the same two-registrations shape done for a font read from disk.
    ///
    /// `None` covers three different reasons a caller cannot tell apart
    /// and does not need to: no embedded program to copy (a bare reference
    /// to one of the standard fourteen), a program `embed::face_name`
    /// cannot name, or no document to register it against. Either way
    /// there is nothing to give another run.
    fn registered_face_for_run(&self, page: usize, object: usize) -> Option<String> {
        let bytes = self.tab().doc.as_ref()?.session.run_font_data(page, object).ok().flatten()?;
        let name = pdf_core::pdf::embed::face_name(&bytes)?;
        if !pdf_core::text::is_registered(&name) {
            pdf_core::text::register(&name, bytes.clone()).ok()?;
        }
        self.tab().doc.as_ref()?.session.add_typing_font(bytes).ok()?;
        Some(name)
    }

    /// The run currently sitting at a baseline origin, addressed by
    /// geometry rather than by a remembered object number.
    ///
    /// **Object numbers are not stable across an edit that adds a page
    /// object.** `TextRun::object` says so directly — "stable until objects
    /// are added or removed" — and embedding a font to satisfy a requested
    /// face (`embed_typing_font`, `set_run_in_stream`'s own last resort)
    /// does exactly that. See `match_font_to_first_selected`'s own doc for
    /// the real corruption this fixed.
    ///
    /// **The baseline origin, not the full rect.** A run's rect grows or
    /// shrinks with its own size or with a font of different glyph widths —
    /// changing either is exactly what this exists to do — so matching on
    /// the *whole box* against a value read before such a change missed the
    /// very run it was looking for and silently skipped it. Where the text
    /// actually *starts* does not move for either reason, only for a move
    /// nothing here ever makes. A small tolerance, not exact equality: this
    /// is comparing PDFium's own re-measurement of the same glyphs against
    /// a value read a moment earlier, not the same float surviving
    /// untouched.
    fn run_at_origin(&self, page: usize, origin: pdf_core::document::Point) -> Option<pdf_core::document::TextRun> {
        const TOLERANCE: f32 = 1.0;
        let runs = self.tab().doc.as_ref()?.session.text_runs(page).ok()?;
        runs.into_iter().find(|r| {
            (r.origin.x - origin.x).abs() < TOLERANCE && (r.origin.y - origin.y).abs() < TOLERANCE
        })
    }

    /// The run standing under one character of a selection — used to find
    /// "the run the selection *starts* in", by its first character.
    ///
    /// **Not `runs_covered_by`.** That answers "which runs does this broad
    /// area touch" by checking whether a run's own *centre* falls inside
    /// the given rects — right for a selection spanning several lines,
    /// wrong here: one character's own tiny box essentially never contains
    /// the centre of the whole run it belongs to. This asks the opposite
    /// question, the same way `text_run_object_at` already does for a
    /// right-click: does *this point* fall inside a run's own rect.
    fn run_at_selection_start(
        &mut self,
        page: usize,
        start: usize,
    ) -> Option<pdf_core::document::TextRun> {
        let rects = self.selection_rects(page, start..start + 1);
        let r = rects.first()?;
        let centre = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let object = self.text_run_object_at(page, AppPoint { x: centre.0 as f64, y: centre.1 as f64 })?;
        self.tab_mut().doc.as_ref()?.session.text_runs(page).ok()?.into_iter().find(|r| r.object == object)
    }

    /// Everything the right-click menu's Join/Split actions need to know,
    /// computed with exactly one `text_runs` read for the whole selection —
    /// not one per question. See [`Self::right_click_text_actions`]'s own
    /// doc for why this is called once, at the click, rather than by the
    /// menu itself: egui redraws an open popup's contents every frame, so a
    /// real few-hundred-run document paid the cost of recomputing this
    /// inline dozens of times a second for as long as the menu stayed open.
    fn compute_right_click_text_actions(&mut self, page: usize, at: AppPoint) -> RightClickTextActions {
        let split_object = self
            .text_run_object_at(page, at)
            .filter(|object| self.group_containing(page, *object).is_some());

        let selection = (page == self.tab_mut().selection_page)
            .then(|| self.tab_mut().text_selection.clone())
            .flatten()
            .filter(|range| !range.is_empty());
        let Some(range) = selection else {
            return RightClickTextActions { joinable: false, split_object };
        };

        let rects = self.selection_rects(page, range.clone());
        let covered = self.runs_touched_by(page, &rects);
        let joinable = covered.len() >= 2;

        RightClickTextActions { joinable, split_object }
    }

    /// Write every rect of a pending link as its own `/Link` annotation, all
    /// carrying the same address — see `Annotation::Link`'s own doc for why
    /// one per line rather than one annotation for the whole selection.
    fn apply_web_link(&mut self, pending: &PendingLink) -> Result<String, String> {
        let typed = pending.url.trim();
        if typed.is_empty() {
            return Err("nothing to link to.".into());
        }
        // A bare "example.com" needs a scheme, or the address opens nowhere
        // in most readers.
        let url = if typed.contains("://") { typed.to_string() } else { format!("https://{typed}") };

        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        let mut made = 0usize;
        for rect in &pending.rects {
            doc.session
                .add_link(pending.page, *rect, url.clone())
                .map_err(|e| e.to_string())?;
            made += 1;
        }
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        // **"there is no indication of it... show it in blue font color but
        // dont change the font style."** An invisible, borderless `/Link`
        // (see that variant's own doc) gives a reader nothing to see short
        // of hovering or clicking — the colour is what a link normally
        // says so with.
        self.colour_linked_text(pending.page, &pending.rects);
        Ok(format!("linked {made} line{} to {url}.", if made == 1 { "" } else { "s" }))
    }

    /// Recolour every run a link's own rects cover to a standard hyperlink
    /// blue — colour only, never the face or size, by sending the run's own
    /// unchanged text back through `SetTextRun` with nothing else in the
    /// style asked for (see `apply_one_edit`'s own doc for why an empty
    /// style is what keeps a change on the byte-safe path that touches
    /// nothing but what was asked for).
    ///
    /// **A run counts as covered by its own centre point**, not by any
    /// partial overlap — the same rule a click already uses to find "the
    /// run at a point" elsewhere in this file. A selection that happened to
    /// start or end mid-run leaves that one run in its original colour
    /// rather than risk colouring text past the actual link.
    /// Every text run on a page whose centre falls inside one of these
    /// rects — the same "is this run part of the selection" test a link's
    /// own recolouring and a manual paragraph join both need, written once.
    fn runs_covered_by(
        &self,
        page: usize,
        rects: &[pdf_core::document::Rect],
    ) -> Vec<pdf_core::document::TextRun> {
        let Some(doc) = &self.tab().doc else { return Vec::new() };
        let Ok(runs) = doc.session.text_runs(page) else { return Vec::new() };
        runs.into_iter()
            .filter(|run| {
                let centre =
                    ((run.rect.left + run.rect.right) / 2.0, (run.rect.top + run.rect.bottom) / 2.0);
                rects.iter().any(|r| {
                    let (left, right) = (r.left.min(r.right), r.left.max(r.right));
                    let (top, bottom) = (r.top.min(r.bottom), r.top.max(r.bottom));
                    centre.0 >= left && centre.0 <= right && centre.1 >= top && centre.1 <= bottom
                })
            })
            .collect()
    }

    /// Every text run a selection genuinely touches — unlike
    /// `runs_covered_by`'s own centre-point test, a run only partially
    /// inside horizontally still counts, since a selection legitimately
    /// starts or ends mid-run.
    ///
    /// **Vertical overlap is held to a much stricter bar than horizontal.**
    /// Text lines sit close enough that two consecutive lines' own boxes
    /// can overlap by a fraction of a point — an ascender or a generous
    /// `line_rects` bound, not a real second line being selected — so a
    /// bare "any overlap at all" test pulled in the line *above* a real
    /// selection on a real page (`"Light output ratio 85%"`, a hair's
    /// breadth above `"...Efficacy 11"`). Requiring most of a run's own
    /// height to fall inside the rect keeps that line out while still
    /// admitting a run only slightly overlapping *sideways* — which is
    /// exactly the shape a mid-run selection has, wide and shallow rather
    /// than narrow and deep.
    ///
    /// Reported from use: a selection spanning "...Efficacy 11" and
    /// "0 lm/w" (the "0" in a visibly different font) never offered "Match
    /// the font" — the first run's own centre sits under "Luminaire
    /// Efficacy", far outside the narrow selection box the drag actually
    /// made, so `runs_covered_by` dropped it and left nothing on that side
    /// of the mismatch to compare against.
    ///
    /// **Wrong for `colour_linked_text`'s own use**, which keeps the
    /// stricter centre rule on purpose — see its own doc for why recolouring
    /// needs the opposite bias.
    fn runs_touched_by(
        &self,
        page: usize,
        rects: &[pdf_core::document::Rect],
    ) -> Vec<pdf_core::document::TextRun> {
        let Some(doc) = &self.tab().doc else { return Vec::new() };
        let Ok(runs) = doc.session.text_runs(page) else { return Vec::new() };
        let touches = |run: &pdf_core::document::Rect, r: &pdf_core::document::Rect| {
            let (a_left, a_right) = (run.left.min(run.right), run.left.max(run.right));
            let (a_top, a_bottom) = (run.top.min(run.bottom), run.top.max(run.bottom));
            let (b_left, b_right) = (r.left.min(r.right), r.left.max(r.right));
            let (b_top, b_bottom) = (r.top.min(r.bottom), r.top.max(r.bottom));
            let h_overlap = a_right.min(b_right) - a_left.max(b_left);
            let v_overlap = a_bottom.min(b_bottom) - a_top.max(b_top);
            let run_height = (a_bottom - a_top).max(1.0);
            h_overlap > 0.0 && v_overlap >= run_height * 0.5
        };
        runs.into_iter().filter(|run| rects.iter().any(|r| touches(&run.rect, r))).collect()
    }

    fn colour_linked_text(&mut self, page: usize, rects: &[pdf_core::document::Rect]) -> usize {
        const LINK_BLUE: pdf_core::document::Color =
            pdf_core::document::Color { r: 5, g: 99, b: 193, a: 255 };
        let runs = self.runs_covered_by(page, rects);
        let Some(doc) = &self.tab_mut().doc else { return 0 };

        let mut coloured = 0usize;
        for run in &runs {
            let recoloured = doc.session.execute(pdf_core::command::Command::SetTextRun {
                page_index: page,
                object: run.object,
                text: run.text.clone(),
                style: pdf_core::document::TextStyle { color: Some(LINK_BLUE), ..Default::default() },
            });
            match recoloured {
                Ok(_) => coloured += 1,
                // **A colour-only change now has its own byte-safe path**
                // (see `set_run_color_in_stream`), which handles almost
                // every run — but that path still has to *find* the run's
                // own text-showing operator by walking the content stream,
                // and a small minority of runs are drawn in a way that walk
                // cannot map back precisely (the same limitation
                // `set_run_in_stream` already has for retyping). Reported
                // from use, on a real document, before the byte-safe path
                // existed: every attempt was refused this way, so the link
                // itself was always there but with no visible way to tell.
                // An underline is the fallback that still says "this is a
                // link" without touching a single byte of the page's own
                // text.
                Err(_) => {
                    let _ = doc.session.execute(pdf_core::command::Command::AddAnnotation {
                        page_index: page,
                        annotation: pdf_core::document::Annotation::Underline {
                            rects: vec![run.rect],
                            color: LINK_BLUE,
                        },
                    });
                }
            }
        }
        // Unconditional: a run that could not be recoloured may still have
        // gained an underline, which needs the same re-render the colour
        // change would have.
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        coloured
    }


    /// Write an Article Box as a visible bordered region, with its title
    /// anchored at the top-left corner as an ordinary note (so it is
    /// readable, and inspectable, without this app's own panel).
    ///
    /// **The simplest honest reading of an obscure Acrobat feature.** A real
    /// PDF article thread (`/Articles`/`/Bead`) spans regions across
    /// possibly many pages and is rendered by almost nothing but Acrobat
    /// itself. A labelled region on the one page it was drawn on is what
    /// every existing annotation type here can already carry, needs no new
    /// engine code, and is the part of the feature actually visible to
    /// opening the file anywhere else.
    fn apply_article_box(&mut self, pending: &PendingArticleBox) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        let r = pending.rect;
        let border = pdf_core::document::Color { r: 94, g: 92, b: 230, a: 255 };
        let outline = vec![
            pdf_core::document::Point { x: r.left, y: r.top },
            pdf_core::document::Point { x: r.right, y: r.top },
            pdf_core::document::Point { x: r.right, y: r.bottom },
            pdf_core::document::Point { x: r.left, y: r.bottom },
            pdf_core::document::Point { x: r.left, y: r.top },
        ];
        doc.session
            .execute(pdf_core::command::Command::AddAnnotation {
                page_index: pending.page,
                annotation: pdf_core::document::Annotation::Ink {
                    strokes: vec![outline],
                    color: border,
                    width: 1.5,
                },
            })
            .map_err(|e| e.to_string())?;

        let title = pending.title.trim();
        if !title.is_empty() {
            let tag = pdf_core::document::Rect {
                left: r.left,
                top: r.top,
                right: r.left + 20.0,
                bottom: r.top + 20.0,
            };
            doc.session
                .note(pending.page, tag, title.to_string(), border)
                .map_err(|e| e.to_string())?;
        }
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        Ok(if title.is_empty() {
            "article box added.".to_string()
        } else {
            format!("article box added — \"{title}\".")
        })
    }


    fn copy_selection(&mut self, ctx: &egui::Context) {
        // The page the selection was made on, not whichever one happens to be
        // in view now.
        let page = self.tab_mut().selection_page;
        let Some(range) = self.tab_mut().text_selection.clone() else {
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

    /// ⌘C for a drawn shape or a placed picture — see [`ObjectClipboard`].
    ///
    /// Tried before [`Self::copy_selection`]'s text path, and only when
    /// [`Focus::allows_document_keys`] says nothing has focus: the two kinds
    /// of selection this app has (a text drag, or a shape/picture picked
    /// with no tool armed) are never live at once, so whichever one is
    /// active is the one ⌘C means. Returns whether it found something to
    /// take, so the caller knows whether to fall back to the text path.
    fn copy_object_selection(&mut self) -> bool {
        let page = self.tab_mut().page;
        if let Some(sel) = self.tab_mut().placed_image_selected.clone().filter(|s| s.page == page) {
            let mark = self.tab_mut()
                .doc
                .as_ref()
                .and_then(|d| d.session.placed_image_marks(page).ok())
                .and_then(|marks| marks.into_iter().find(|m| m.index == sel.index));
            let Some(mark) = mark else { return false };
            self.object_clipboard = Some(ObjectClipboard::Image {
                rgba: mark.rgba,
                width: mark.width,
                height: mark.height,
                rect: mark.rect,
            });
            self.paste_count = 0;
            self.clipboard_mirror_wanted = true;
            self.forget_copied_pages();
            self.say_info("picture copied — `paste` puts a copy down.");
            return true;
        }
        if let Some(layer) = self.tab_mut().markup.existing(page) {
            if !layer.selection().is_empty() {
                // A Hatch is never a shape someone meant to copy on its own —
                // it is the fill's own bookkeeping, carried instead through
                // `is_filled` below. Left in, a `select all` sweep (which,
                // unlike a click or a box, does not filter to what is
                // independently selectable) would copy it as if it were a
                // shape and leave the paste pointing at the original's
                // handle instead of its own.
                let objects: Vec<(cad_kernel::DObject, bool)> = layer
                    .selection()
                    .iter()
                    .filter_map(|&i| layer.objects().get(i).map(|o| (i, o)))
                    .filter(|(_, o)| !matches!(o.geom, cad_kernel::Geom::Hatch { .. }))
                    .map(|(i, o)| (o.clone(), layer.is_filled(i)))
                    .collect();
                if !objects.is_empty() {
                    let n = objects.len();
                    self.object_clipboard = Some(ObjectClipboard::Shapes(objects));
                    self.paste_count = 0;
                    self.clipboard_mirror_wanted = true;
                    self.forget_copied_pages();
                    self.say_info(format!(
                        "{n} shape{} copied — `paste` puts {} down.",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "a copy" } else { "copies" }
                    ));
                    return true;
                }
            }
        }
        // A picture, shape or run of words picked with Edit Object.
        if let Some(sel) = self.tab().selected.clone().filter(|s| s.page == page) {
            return self.copy_page_object(&sel);
        }
        false
    }

    /// Fill the clipboard from a copy that is not text, and say how to put it
    /// down.
    fn put_on_clipboard(&mut self, content: ObjectClipboard, said: String) {
        self.object_clipboard = Some(content);
        self.paste_count = 0;
        self.clipboard_mirror_wanted = true;
        self.forget_copied_pages();
        self.say_info(said);
    }

    /// ⌘C on what Edit Object has picked.
    ///
    /// **Words are copied as words** — their text, size, colour and font — and
    /// paste back as new, editable text. A picture or shape the page itself
    /// draws cannot be lifted out of the page's content stream, so it is
    /// copied as what it looks like: the page's own pixels over its box, the
    /// paper around a shape made see-through (see [`clear_paper_around`]) so
    /// that it can be set down over other things.
    fn copy_page_object(&mut self, sel: &Selected) -> bool {
        let page = sel.page;
        let content = if sel.what == "the words" {
            let run = self
                .tab()
                .doc
                .as_ref()
                .and_then(|d| d.session.text_runs(page).ok())
                .and_then(|runs| runs.into_iter().find(|r| r.object == sel.object));
            let Some(run) = run else { return false };
            let face = self.registered_face_of_object(page, sel.object);
            ObjectClipboard::Text { lines: vec![run.text.trim().to_string()], size: run.size, color: run.color, face }
        } else {
            let raster = self.tab().doc.as_ref().and_then(|d| d.session.render_page_region(page, sel.rect, 3.0).ok());
            let Some(raster) = raster else { return false };
            let mut rgba = raster.pixels;
            if sel.what == "the shape" {
                clear_paper_around(&mut rgba, raster.width as usize, raster.height as usize);
            }
            ObjectClipboard::Image { rgba, width: raster.width, height: raster.height, rect: sel.rect }
        };
        let what = sel.what.strip_prefix("the ").unwrap_or(sel.what);
        self.put_on_clipboard(content, format!("{what} copied — Ctrl+V picks it up, a click puts it down."));
        true
    }

    /// ⌘C in the run/paragraph editor with **nothing selected inside its box**:
    /// the whole text object is the thing copied (with a selection, the box's
    /// own copy of the characters is what is meant, and is left alone).
    fn copy_editing_run(&mut self, ctx: &egui::Context) -> bool {
        let Some(edit) = self.tab().editing_run.clone() else { return false };
        let id = egui::Id::new(("run-editor", edit.page, edit.object));
        let has_selection = egui::TextEdit::load_state(ctx, id)
            .and_then(|state| state.cursor.char_range())
            .is_some_and(|range| !range.is_empty());
        if has_selection || edit.buffer.trim().is_empty() {
            return false;
        }
        let lines: Vec<String> = edit.buffer.split('\n').map(|line| line.trim_end().to_string()).collect();
        let content = ObjectClipboard::Text {
            lines,
            size: edit.style.size.or(edit.was.size).unwrap_or(12.0),
            color: edit.style.color.or(edit.was.color).unwrap_or(pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 }),
            face: self.registered_face_of(&edit),
        };
        self.put_on_clipboard(content, "text copied — Ctrl+V picks it up, a click puts it down.".into());
        true
    }

    /// ⌘V in Edit Object / Edit Text: **pick the paste up instead of putting it
    /// down.** It follows the pointer at half strength ([`Self::draw_paste_ghost`])
    /// and goes where the next click on a page lands ([`Self::place_paste_ghost`]).
    /// `pasted` is the text the system clipboard held: anything but our own
    /// placeholder means words were copied since, somewhere else, and those win.
    /// `false` when this is not the time (no such tool in hand) or there is
    /// nothing to paste, and the caller pastes the old way.
    fn start_paste_ghost(&mut self, pasted: Option<String>) -> bool {
        let in_hand = self.tab().object_tool.is_some()
            || self.tab().editing_run.is_some()
            || self.tab().tool.as_ref().is_some_and(|p| matches!(p.kind, Tool::PickText));
        if !in_hand {
            return false;
        }
        let from_elsewhere = pasted.filter(|text| text != COPIED_IN_PAGIFY && !text.trim().is_empty());
        let content = match from_elsewhere {
            Some(text) => ObjectClipboard::Text {
                lines: text.lines().map(str::to_string).collect(),
                size: 12.0,
                color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
                face: None,
            },
            None => match self.object_clipboard.clone() {
                Some(content) => content,
                None => return false,
            },
        };
        self.paste_ghost = Some(PasteGhost { content, texture: None });
        self.say_info("pasting — move to where it goes and click to put it down. Escape cancels.");
        true
    }

    /// The click that puts a picked-up paste down, with the pointer at `at`:
    /// words start at the pointer, a picture or shapes are centred on it.
    /// One undo step, however many lines.
    fn place_paste_ghost(&mut self, page: usize, at: AppPoint) {
        let Some(ghost) = self.paste_ghost.take() else { return };
        match ghost.content {
            ObjectClipboard::Text { lines, size, color, face } => {
                let gap = size * 1.2;
                let mut baseline = at.y as f32 + size * 0.8;
                let mut commands = Vec::new();
                for line in &lines {
                    if !line.trim().is_empty() {
                        match self.styled_line_command(page, (at.x as f32, baseline), line, size, color, face.as_deref()) {
                            Ok(command) => commands.push(command),
                            Err(e) => {
                                self.say_error(e);
                                return;
                            }
                        }
                    }
                    baseline += gap;
                }
                let n = commands.len();
                let command = match commands.len() {
                    0 => {
                        self.say_info("nothing to paste.");
                        return;
                    }
                    1 => commands.remove(0),
                    _ => pdf_core::command::Command::Batch { commands },
                };
                let outcome = self.tab().doc.as_ref().map(|d| d.session.execute(command));
                match outcome {
                    Some(Ok(_)) => {
                        if let Some(doc) = &mut self.tab_mut().doc {
                            doc.rendered_is_stale();
                        }
                        self.say_info(format!("text pasted ({n} line{}) — `undo` takes it back.", if n == 1 { "" } else { "s" }));
                    }
                    Some(Err(e)) => self.say_error(format!("{e}")),
                    None => self.say_error("nothing open."),
                }
            }
            ObjectClipboard::Image { rgba, width, height, rect } => {
                let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
                let rect = pdf_core::document::Rect {
                    left: at.x as f32 - w / 2.0,
                    right: at.x as f32 + w / 2.0,
                    top: at.y as f32 - h / 2.0,
                    bottom: at.y as f32 + h / 2.0,
                };
                let Some(doc) = &self.tab().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                let outcome = doc.session.execute(pdf_core::command::Command::AddAnnotation {
                    page_index: page,
                    annotation: pdf_core::document::Annotation::Image { rect, rgba, width, height },
                });
                match outcome {
                    Ok(_) => {
                        if let Some(doc) = &mut self.tab_mut().doc {
                            doc.rendered_is_stale();
                        }
                        self.say_info("picture pasted — `undo` takes it back.");
                    }
                    Err(e) => self.say_error(format!("{e}")),
                }
            }
            ObjectClipboard::Shapes(objects) => {
                let height = view_height(self, page);
                let layer = self.tab_mut().markup.page(page, height);
                let centre = shapes_centre(&objects, layer);
                let by = cad_kernel::Vec2::new(at.x - centre.x, -(at.y - centre.y));
                self.put_shapes_down(page, &objects, by);
            }
        }
    }

    /// Copies of `objects`, each moved by `by` (kernel space, which counts
    /// upwards), added to the page's markup and left selected.
    fn put_shapes_down(&mut self, page: usize, objects: &[(cad_kernel::DObject, bool)], by: cad_kernel::Vec2) {
        let height = view_height(self, page);
        let layer = self.tab_mut().markup.page(page, height);
        layer.begin("paste");
        layer.clear_selection();
        let mut made = Vec::new();
        for (object, was_filled) in objects {
            let copy = cad_kernel::DObject { handle: cad_kernel::next_handle(), ..object.translated(by) };
            made.push((layer.add_object(copy), *was_filled));
        }
        let n = made.len();
        for (index, was_filled) in made {
            layer.select_box_index(index);
            if was_filled {
                layer.set_filled(index, true);
            }
        }
        layer.end();
        self.say_info(format!("{n} shape{} pasted.", if n == 1 { "" } else { "s" }));
    }

    /// The picked-up paste, drawn under the pointer at 50% opacity while the
    /// pointer is over this page.
    fn draw_paste_ghost(&mut self, ui: &mut egui::Ui, page: usize, rect: egui::Rect, view: PageView) {
        let Some(pointer) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) else { return };
        let painter = ui.painter_at(rect);
        // Taken out while it is drawn (the shapes need the markup layer, which
        // is the app's too) and put straight back.
        let Some(mut ghost) = self.paste_ghost.take() else { return };
        match &ghost.content {
            ObjectClipboard::Text { lines, size, color, .. } => {
                let font = egui::FontId::proportional((size * view.scale).max(4.0));
                let ink = egui::Color32::from_rgb(color.r, color.g, color.b).gamma_multiply(0.5);
                for (i, line) in lines.iter().enumerate() {
                    let at = pointer + egui::vec2(0.0, i as f32 * size * 1.2 * view.scale);
                    painter.text(at, egui::Align2::LEFT_TOP, line, font.clone(), ink);
                }
            }
            ObjectClipboard::Image { rgba, width, height, rect: source } => {
                let texture = ghost.texture.get_or_insert_with(|| {
                    let image = egui::ColorImage::from_rgba_unmultiplied([*width as usize, *height as usize], rgba);
                    ui.ctx().load_texture("paste-ghost", image, egui::TextureOptions::LINEAR)
                });
                let size = egui::vec2((source.right - source.left) * view.scale, (source.bottom - source.top) * view.scale);
                painter.image(
                    texture.id(),
                    egui::Rect::from_center_size(pointer, size),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha(128),
                );
            }
            ObjectClipboard::Shapes(objects) => {
                let at = view.to_page(pointer);
                let height = view_height(self, page);
                let layer = self.tab_mut().markup.page(page, height);
                let centre = shapes_centre(objects, layer);
                let by = cad_kernel::Vec2::new(at.x - centre.x, -(at.y - centre.y));
                let ink = egui::Color32::from_rgb(40, 40, 40).gamma_multiply(0.5);
                overlay::draw_ghost(&painter, objects, layer, view, by, ink);
            }
        }
        self.paste_ghost = Some(ghost);
    }

    /// How far each successive paste is offset from where it was copied —
    /// far enough to see there are now two, close enough that it is still
    /// obviously the paste. Stepped by [`Self::paste_count`] rather than
    /// fixed, so pasting twice does not stack the second exactly on the
    /// first and look like nothing happened.
    const PASTE_STEP: f64 = 18.0;

    /// ⌘V for whatever `copy` last took — see [`Self::copy_object_selection`].
    fn paste_object_selection(&mut self) {
        let Some(clip) = self.object_clipboard.clone() else {
            self.say_info("nothing to paste — `copy` a shape or picture first.");
            return;
        };
        self.paste_count += 1;
        let step = Self::PASTE_STEP * self.paste_count as f64;
        let page = self.tab_mut().page;

        match clip {
            // Words have no place of their own to be copied beside: they are
            // picked up and put down where the reader clicks.
            ObjectClipboard::Text { .. } => {
                self.paste_ghost = Some(PasteGhost { content: clip, texture: None });
                self.say_info("pasting — move to where it goes and click to put it down. Escape cancels.");
            }
            ObjectClipboard::Shapes(objects) => {
                // Kernel space counts upwards — see `finish_markup_grab` —
                // so "down and to the right" on the page is `(+, -)` here.
                self.put_shapes_down(page, &objects, cad_kernel::Vec2::new(step, -step));
            }
            ObjectClipboard::Image { rgba, width, height, rect } => {
                let Some(doc) = &self.tab_mut().doc else {
                    self.say_error("nothing open.");
                    return;
                };
                let offset = step as f32;
                let rect = pdf_core::document::Rect {
                    left: rect.left + offset,
                    right: rect.right + offset,
                    top: rect.top + offset,
                    bottom: rect.bottom + offset,
                };
                let outcome = doc.session.execute(pdf_core::command::Command::AddAnnotation {
                    page_index: page,
                    annotation: pdf_core::document::Annotation::Image { rect, rgba, width, height },
                });
                match outcome {
                    Ok(_) => {
                        if let Some(doc) = &mut self.tab_mut().doc {
                            doc.rendered_is_stale();
                        }
                        self.say_info("picture pasted.");
                    }
                    Err(e) => self.say_error(format!("{e}")),
                }
            }
        }
    }

    /// Copy the Organize grid's own current selection — see
    /// [`PagifyApp::page_clipboard`]. Replaces whatever was copied before
    /// outright, the same way [`Self::copy_object_selection`] does for
    /// shapes and pictures.
    fn copy_organize_selection(&mut self) -> bool {
        let mut pages: Vec<usize> = self.tab_mut().organize_selected.clone();
        pages.sort_unstable();
        if pages.is_empty() {
            return false;
        }
        // **A clipboard every Pagify window can paste from.** Reported from
        // use: "page copy and paste only works between two files opened in the
        // same window; opened separately, it doesn't paste". The clipboard was
        // a file named after this process, remembered only by this process.
        // It is now a uniquely named file in one shared folder plus a small
        // manifest naming the newest one ([`Self::shared_page_clipboard`]), so
        // the window that pastes need not be the window that copied — nor still
        // be open. A copy replaces the previous one for everybody, which is
        // what a clipboard is; the earlier files are cleared as it is made.
        static COPIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let _ = std::fs::create_dir_all(&self.clipboard_dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let temp_file = self.clipboard_dir.join(format!(
            "pages-{stamp}-{}-{}.pdf",
            std::process::id(),
            COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        self.page_clipboard = None;
        let result = match &self.tab_mut().doc {
            Some(doc) => doc.session.extract_to(&pages, &temp_file),
            None => return false,
        };
        match result {
            Ok(page_count) => {
                let n = pages.len();
                Self::publish_page_clipboard(&self.clipboard_dir, &temp_file, page_count);
                self.page_clipboard = Some(PageClipboard { temp_file, page_count });
                self.clipboard_mirror_wanted = true;
                self.say_info(format!(
                    "{n} page{} copied — `paste` puts {} in.",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "it" } else { "them" }
                ));
                true
            }
            Err(e) => {
                self.say_error(format!("{e}"));
                false
            }
        }
    }

    /// The folder every Pagify window leaves copied pages in, and reads them
    /// from. Under the system's temp folder, and the same for every window of
    /// the same user — a test gives each app its own, so tests cannot paste
    /// each other's pages.
    fn shared_clipboard_dir() -> std::path::PathBuf {
        std::env::temp_dir().join("pagify-clipboard")
    }

    /// Name the newest copy in the folder's manifest (`latest.txt`: its file
    /// name, then its page count) and clear every earlier copy. The manifest is
    /// written beside itself and renamed into place, so a window reading it
    /// never sees half of one. Best-effort throughout: a failure here costs a
    /// paste in another window, never the copy itself.
    fn publish_page_clipboard(dir: &std::path::Path, file: &std::path::Path, page_count: usize) {
        let Some(name) = file.file_name().and_then(|n| n.to_str()) else { return };
        let staged = dir.join("latest.tmp");
        if std::fs::write(&staged, format!("{name}\n{page_count}\n")).is_ok() {
            let _ = std::fs::rename(&staged, dir.join("latest.txt"));
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let other = entry.file_name();
                let other = other.to_string_lossy();
                if other.starts_with("pages-") && other.ends_with(".pdf") && other != name {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    /// The newest copy any window made, if it is still there and recent: the
    /// file, and how many pages it holds. An hour is as long as a copy is
    /// offered to a window that did not make it — a stale one must not answer a
    /// paste meant for something else.
    fn shared_page_clipboard(dir: &std::path::Path) -> Option<(std::path::PathBuf, usize)> {
        let manifest = dir.join("latest.txt");
        let age = std::fs::metadata(&manifest).ok()?.modified().ok()?.elapsed().ok()?;
        if age > std::time::Duration::from_secs(3600) {
            return None;
        }
        let text = std::fs::read_to_string(&manifest).ok()?;
        let mut lines = text.lines();
        let file = dir.join(lines.next()?.trim());
        let count = lines.next()?.trim().parse::<usize>().ok()?;
        (count > 0 && file.is_file()).then_some((file, count))
    }

    /// Copying a shape or a picture replaces whatever pages were copied: there
    /// is one clipboard, and `paste` must paste the newest thing copied — in
    /// any window, which is why the manifest goes too.
    fn forget_copied_pages(&mut self) {
        self.page_clipboard = None;
        let _ = std::fs::remove_file(self.clipboard_dir.join("latest.txt"));
    }

    /// What `paste` would paste right now: the newest copy from any window,
    /// else this window's own (a copy this window made that has since aged out
    /// of the folder is still its own to paste).
    fn current_page_clipboard(&self) -> Option<(std::path::PathBuf, usize)> {
        Self::shared_page_clipboard(&self.clipboard_dir)
            .or_else(|| self.page_clipboard.as_ref().map(|c| (c.temp_file.clone(), c.page_count)))
    }

    /// Paste Organize's own clipboard into the current tab — the same
    /// document it was copied from, or a different one entirely. Lands
    /// right above (before) whichever page is currently selected, or at the
    /// end of the document when nothing is selected.
    fn paste_organize_selection(&mut self) {
        let Some((temp_file, page_count)) = self.current_page_clipboard() else {
            self.say_info("nothing to paste — copy some pages first.");
            return;
        };
        let at = match self.tab_mut().organize_selected.iter().copied().min() {
            Some(p) => p,
            None => self.tab_mut().doc.as_ref().map(|d| d.page_count).unwrap_or(0),
        };
        let result = match &self.tab_mut().doc {
            Some(doc) => doc.session.import_from(&temp_file, &(0..page_count).collect::<Vec<_>>(), at),
            None => {
                self.say_error("nothing open.");
                return;
            }
        };
        match result {
            Ok(_) => self.after_page_change(format!(
                "{page_count} page{} pasted.",
                if page_count == 1 { "" } else { "s" }
            )),
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Flip between the dark and light palettes. `theme::toggle` only flips
    /// the flag and saves it — the repaint happens next frame, from
    /// `theme::apply(&ctx)` at the top of `fn ui`, since this has no context
    /// of its own to call it with.
    fn toggle_appearance(&mut self) {
        theme::toggle();
        self.say_info(match theme::mode() {
            theme::Mode::Light => "light theme.",
            theme::Mode::Dark => "dark theme.",
        });
    }

    /// A zero-based page index list, written the way every page-spec-taking
    /// verb function (`delete_pages`, `duplicate_pages`, `rotate_pages`, …)
    /// expects to read it back — one-based, comma-separated. The one place
    /// Organize's own selection state meets that older, typed-command
    /// surface, so every operation the grid offers reuses the existing,
    /// already-tested verb instead of a new index-based twin of it.
    fn page_spec(pages: &[usize]) -> String {
        pages.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(",")
    }

    /// Show or hide the wider Organize grid for whichever tab is active —
    /// see [`PagifyApp::organize_open`]. This is only a bigger view: the
    /// page selection it shows is the same one the plain Pages rail uses,
    /// so closing the grid leaves the selection (and an in-progress drag)
    /// alone rather than dropping it — reorganizing pages keeps working in
    /// the rail either way.
    fn toggle_organize_grid(&mut self) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        self.organize_open = !self.organize_open;
        if self.organize_open {
            self.say_info("Organize — click a page to select it, drag to reorder, Escape to close.");
        } else {
            self.say_info("Organize closed — the Pages rail selects, drags, copies and pastes pages too.");
        }
    }

    /// Delete the Organize grid's own current selection.
    fn delete_organize_selection(&mut self) {
        let mut pages = std::mem::take(&mut self.tab_mut().organize_selected);
        if pages.is_empty() {
            return;
        }
        pages.sort_unstable();
        let spec = Self::page_spec(&pages);
        self.tab_mut().organize_anchor = None;
        self.delete_pages(&spec);
    }

    /// How many pages of about [`GRID_CELL_PT`] fit across `width` points of
    /// the Organize grid, with [`GRID_GAP_PT`] between them. Never fewer than
    /// one. Pure, like [`Self::nearest_drop`] below, so it can be tested
    /// without a running UI.
    fn grid_columns(width: f32) -> usize {
        (((width + GRID_GAP_PT) / (GRID_CELL_PT + GRID_GAP_PT)).floor() as usize).max(1)
    }

    /// How wide each of `columns` pages is drawn so that a row of them, and
    /// the gaps between, **is never wider than `width`** — not by a point.
    /// The panel the grid sits in is as wide as the grid asks to be, so a row
    /// that overshoots its width by anything makes the panel wider, which
    /// makes the pages wider, which overshoots again: see
    /// [`Self::draw_organize_grid`]. The half point is for rounding.
    fn grid_cell_width(width: f32, columns: usize) -> f32 {
        ((width - GRID_GAP_PT * (columns as f32 - 1.0)) / columns as f32 - 0.5).max(GRID_CELL_MIN_PT)
    }

    /// Where a drag-to-reorder would drop, and where to draw the indicator
    /// for it — the cell boundary nearest the pointer among this frame's own
    /// cell rects (page index, its own on-screen rect). Returns the
    /// zero-based `before` index [`pagify_shell::organize::order_for_move`]
    /// wants, the x coordinate to draw the indicator line at, and the y
    /// range to draw it across.
    ///
    /// Pure and free of `self`/`egui::Ui` on purpose — nothing else about an
    /// egui drag can be exercised without a running UI; this can be, and is,
    /// tested directly.
    fn nearest_drop(
        cells: &[(usize, egui::Rect)],
        pointer: egui::Pos2,
    ) -> Option<(usize, f32, egui::Rangef)> {
        let &(nearest_page, nearest_rect) = cells
            .iter()
            .min_by(|(_, a), (_, b)| a.center().distance(pointer).total_cmp(&b.center().distance(pointer)))?;
        let before = if pointer.x < nearest_rect.center().x { nearest_page } else { nearest_page + 1 };
        let indicator_x =
            cells.iter().find(|(p, _)| *p == before).map(|(_, r)| r.left()).unwrap_or(nearest_rect.right());
        Some((before, indicator_x, nearest_rect.y_range()))
    }

    /// What a click on page `page` does to the Organize grid's own
    /// selection — plain click selects just it, ctrl+click toggles it
    /// without touching the rest, shift+click selects the contiguous range
    /// from `anchor` (the standard file-manager range-select idiom; the
    /// canvas's own shift-click, [`Self::extend_selection_at`], only
    /// toggles one item at a time, because a canvas selection has no
    /// natural order to span the way pages do).
    ///
    /// Pure and free of `self`/`egui::Ui` on purpose, same reasoning as
    /// [`Self::nearest_drop`] right above.
    fn apply_organize_click(
        selected: &mut Vec<usize>,
        anchor: &mut Option<usize>,
        page: usize,
        modifiers: egui::Modifiers,
    ) {
        if modifiers.command {
            if let Some(at) = selected.iter().position(|&p| p == page) {
                selected.remove(at);
            } else {
                selected.push(page);
            }
            *anchor = Some(page);
        } else if modifiers.shift {
            let from = anchor.unwrap_or(page);
            let (lo, hi) = (from.min(page), from.max(page));
            *selected = (lo..=hi).collect();
        } else {
            *selected = vec![page];
            *anchor = Some(page);
        }
    }

    /// Draws one interactive thumbnail — selection (plain/ctrl/shift-click),
    /// jump-to-page, and the start of a drag-to-reorder — shared by the
    /// always-visible "Pages" rail and the wider Organize grid, so
    /// reorganizing pages (select, drag-reorder, copy, paste, delete) never
    /// requires switching into a separate mode first; whichever one is on
    /// screen for the active tab, while any tool is armed, behaves the same
    /// way. Reuses [`Self::thumb_for`]'s own texture cache; nothing about
    /// how a thumbnail is rendered changes here, only what clicking and
    /// dragging it do. Returns the page to jump to, if a plain click landed
    /// on one.
    ///
    /// **Reported from use**: clicking or dragging a thumbnail did not let
    /// Ctrl+C/Ctrl+V/Delete reach the selection at all — every one of them
    /// is gated on nothing having keyboard focus (`Focus::
    /// allows_document_keys`), and the command box — focused by default, or
    /// left focused after typing a command — keeps that focus until
    /// something explicitly takes it away. `fn interact`'s own canvas click
    /// does exactly this for the same reason: a click here is just as
    /// unambiguously aimed at the document as one on the page is.
    /// A page's size as a short, human label — "A4", "A1 Landscape", or the
    /// raw millimetres when it matches no standard sheet, the same style the
    /// mockup's own thumbnail cards use (`code.html:333`, `:344`, `:355`).
    /// Pure: point dimensions in, a label out — tested directly rather than
    /// only through a rendered thumbnail.
    fn paper_size_label(width_pt: f32, height_pt: f32) -> String {
        const TOLERANCE_PT: f32 = 3.0;
        const SIZES: &[(&str, f32, f32)] = &[
            ("A0", 2384.0, 3370.0),
            ("A1", 1684.0, 2384.0),
            ("A2", 1190.0, 1684.0),
            ("A3", 842.0, 1190.0),
            ("A4", 595.0, 842.0),
            ("A5", 420.0, 595.0),
            ("Letter", 612.0, 792.0),
            ("Legal", 612.0, 1008.0),
            ("Tabloid", 792.0, 1224.0),
        ];
        let (short, long) = (width_pt.min(height_pt), width_pt.max(height_pt));
        let landscape = width_pt > height_pt;
        for (name, portrait_short, portrait_long) in SIZES {
            if (short - portrait_short).abs() <= TOLERANCE_PT && (long - portrait_long).abs() <= TOLERANCE_PT {
                return if landscape { format!("{name} Landscape") } else { (*name).to_string() };
            }
        }
        // Millimetres, the unit everyone sizing a real sheet already thinks
        // in — not points, which nothing outside a PDF's own internals uses.
        let mm = |pt: f32| pt / 72.0 * 25.4;
        format!("{:.0}×{:.0}mm", mm(width_pt), mm(height_pt))
    }


    /// Draws the drop-indicator line for a drag-to-reorder in progress —
    /// shared by the rail and the grid, same reasoning as
    /// [`Self::draw_thumbnail_cell`].
    fn draw_drop_indicator(
        ui: &mut egui::Ui,
        dragging: bool,
        cell_rects: &[(usize, egui::Rect)],
        pointer_pos: Option<egui::Pos2>,
    ) {
        if !dragging {
            return;
        }
        if let Some(pointer) = pointer_pos {
            if let Some((_, indicator_x, y_range)) = Self::nearest_drop(cell_rects, pointer) {
                ui.painter().vline(indicator_x, y_range, egui::Stroke::new(3.0, theme::violet()));
            }
        }
    }

    /// Finishes a drag-to-reorder gesture started by
    /// [`Self::draw_thumbnail_cell`] — shared by the rail and the grid.
    fn finish_thumbnail_drag(
        &mut self,
        cell_rects: &[(usize, egui::Rect)],
        pointer_pos: Option<egui::Pos2>,
        released: bool,
    ) {
        if !released {
            return;
        }
        let Some(drag) = self.tab_mut().organize_drag.take() else { return };
        let Some(pointer) = pointer_pos else { return };
        if pointer.distance(drag.pointer_started_at) < Self::MIN_DRAG_PX {
            return;
        }
        let Some((before, _, _)) = Self::nearest_drop(cell_rects, pointer) else { return };
        let count = self.tab_mut().doc.as_ref().map(|d| d.page_count).unwrap_or(0);
        let order = match pagify_shell::organize::order_for_move(count, &drag.moving, before) {
            Ok(order) => order,
            Err(e) => {
                self.say_error(e);
                return;
            }
        };
        let result = match &self.tab_mut().doc {
            Some(doc) => doc.session.execute(pdf_core::command::Command::ReorderPages { order }),
            None => return,
        };
        match result {
            Ok(_) => self.after_page_change("reordered.".to_string()),
            Err(e) => self.say_error(format!("{e}")),
        }
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

    /// Ctrl+P — the native Print dialog on Windows. Not yet available on
    /// the other two platforms this builds for; see `print_windows`'s own
    /// doc for why printing specifically needs real OS integration rather
    /// than just more rendering code, which is the one thing this app
    /// already does the same way everywhere.
    fn print_current_document(&mut self, frame: &mut eframe::Frame) {
        let Some(doc) = &self.tab_mut().doc else {
            self.say_error("nothing open.");
            return;
        };
        let page_count = doc.page_count;
        let title = doc
            .session
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Pagify document".to_string());
        #[cfg(target_os = "windows")]
        let session = doc.session.clone();
        // `doc`'s own borrow ends here — nothing below reads from it again.

        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::HasWindowHandle as _;
            let current_page = self.tab_mut().page;
            let hwnd = match frame.window_handle().map(|h| h.as_raw()) {
                Ok(raw_window_handle::RawWindowHandle::Win32(win32)) => {
                    windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut std::ffi::c_void)
                }
                Ok(_) => {
                    self.say_error("this window has no HWND to print from.");
                    return;
                }
                Err(e) => {
                    self.say_error(format!("couldn't find this window to print from: {e}"));
                    return;
                }
            };
            // `frame` is the first window's. With several windows the one to
            // print from is the one the person is using — see `hub`.
            let hwnd = hub::active_window().unwrap_or(hwnd);
            if let Err(e) = print_windows::print_document(hwnd, &session, page_count, current_page, &title) {
                self.say_error(e);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (frame, page_count, title);
            self.say_info("printing isn't available on this platform yet.");
        }
    }

    /// **Reported from use: there is no Save As button.** `saveas` already
    /// existed as a typed command, but only with a path spelt out by hand —
    /// nothing offered the native file picker `open`'s own ribbon button
    /// gets, and bare `saveas` used to just refuse ("usage: saveas
    /// <path.pdf>") rather than opening one. Mirrors `open_dialog` exactly,
    /// suggesting the current document's own name and folder as a starting
    /// point rather than an empty one.
    fn save_as_dialog(&mut self) {
        let Some(doc) = &self.tab_mut().doc else {
            self.say_error("nothing open.");
            return;
        };
        let current = doc.session.path().to_path_buf();

        let mut dialog = rfd::FileDialog::new().set_title("Save As").add_filter("PDF", &["pdf"]);
        if let Some(dir) = current.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = current.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }

        match dialog.save_file() {
            Some(path) => {
                self.save(Some(path));
            }
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
                if !cfg!(test) {
                    if !cfg!(test) {
            self.outlined_fonts.save();
        }
                }
                self.say_info("font added — tried on outlined pages from now on.");
            }
            Err(why) => self.say_error(why),
        }
    }

    fn remove_outlined_font(&mut self, path: PathBuf) {
        self.outlined_fonts.remove(&path);
        if !cfg!(test) {
            self.outlined_fonts.save();
        }
        self.say_info("removed.");
    }

    fn clear_outlined_fonts(&mut self) {
        self.outlined_fonts.clear();
        if !cfg!(test) {
            self.outlined_fonts.save();
        }
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

        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            if asked {
                self.say_error("nothing open.");
            }
            return;
        };
        let page = self.tab().page;

        let Ok(verdict) = session.classify(page) else {
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
                     real text can be found — `extracttext` reads the rest with OCR and \
                     makes it selectable too.",
                    verdict.glyph_paths, verdict.chars
                ),
            ),
            PageTextKind::Scanned => (
                true,
                "this page has no text layer — it is a picture of text. Selection and \
                 search will find nothing until `extracttext` reads it with OCR."
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
                     Type converted to outlines, as print production does. `extracttext` \
                     reads it with OCR and makes it selectable.",
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
        let reflow_hint = self.tab()
            .doc
            .as_ref()
            .and_then(|d| d.session.with_engine(|s| s.document.page(page)?.glyphs()).ok())
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

    /// The frame a page is drawn in — turned with the view. The strip already
    /// holds it, so there is nothing left to swap here: it used to swap on top
    /// of a strip that did not, which is the other half of the same bug.
    fn page_extent(&self, page: usize) -> (f32, f32) {
        let Some(doc) = &self.tab().doc else { return (612.0, 792.0) };
        doc.strip.frame_of(page).unwrap_or((612.0, 792.0))
    }

    // -- running verbs ------------------------------------------------------

    /// Run a command line as though it had been typed and submitted.
    pub fn submit(&mut self, line: &str) {
        // A run open for editing takes the line: the words being typed are
        // text, not a command, and this is the path tests and recordings use in
        // place of typing into the box on the page.
        if let Some(edit) = self.tab_mut().editing_run.as_mut() {
            edit.buffer = line.to_string();
            // **Not kept open when the engine refuses it**, as an apply from the page
            // is: nobody is at the box to correct the words — this is a script, a
            // recording or a test — and the line after this one must run as the command
            // it is, not be taken for more words to type into a box left open.
            if self.apply_editing_page() {
                self.tab_mut().editing_run = None;
            }
            return;
        }
        if self.tab_mut().awaiting_password.is_some() {
            self.cmd.input_mut().clear();
            self.cmd.input_mut().push_str(line);
            self.consume_password_line();
            return;
        }
        if let Some(dispatch) = pagify_shell::command::dispatch(line) {
            self.cmd.say(Kind::Echo, line);
            self.recorder.observe(line);
            self.session_log.record("command", line);
            self.run(dispatch);
        }
    }



    fn draw(&mut self, command: cad_kernel::parser::Command) {
        let page = self.tab().page;
        let Some(doc) = self.tab().doc.as_ref() else {
            self.say_error("open a document before drawing on it.");
            return;
        };
        let height = doc.strip.size_of(page).map(|(_, h)| h as f64).unwrap_or(792.0);

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

        let mut defaults = std::mem::take(&mut self.defaults);
        let layer = self.tab_mut().markup.page(page, height);
        let applied = tools::apply(layer, &command, &mut defaults);
        self.defaults = defaults;

        match applied {
            tools::Applied::Added(n) => self.say_info(format!("{n} added.")),
            tools::Applied::Changed(n) => self.say_info(format!("{n} changed.")),
            tools::Applied::Removed(n) => self.say_info(format!("{n} removed.")),
            // The Eraser with nothing drawn selected picks the tool up for
            // highlights and other text marks (see `erase_mark_at`) instead of
            // just saying nothing is selected.
            tools::Applied::Nothing(_) if matches!(command, cad_kernel::parser::Command::DeleteSelected) => {
                self.arm(Tool::EraseMark, page);
            }
            tools::Applied::Nothing(why) => self.say_info(why),
            tools::Applied::Failed(why) => self.say_error(why),

            tools::Applied::Tool(kind) => {
                use cad_kernel::parser::ToolKind;
                let draw = match kind {
                    ToolKind::Line => Some(DrawKind::Line),
                    ToolKind::Circle => Some(DrawKind::Circle),
                    ToolKind::Rectangle => Some(DrawKind::Rectangle),
                    ToolKind::Polyline => Some(DrawKind::Polyline),
                    ToolKind::Spline => Some(DrawKind::Spline),
                    _ => None,
                };
                match draw {
                    Some(kind) => self.arm(Tool::Draw(kind), page),
                    None => self.say_info(
                        "that tool draws from typed coordinates for now — e.g. `arc3p 0,0 50,50 100,0`.",
                    ),
                }
            }

            tools::Applied::Interactive(pick) => {
                if pick.needs_selection {
                    let empty = self.tab_mut()
                        .markup
                        .existing(page)
                        .map(|l| l.selection().is_empty())
                        .unwrap_or(true);
                    if empty {
                        self.say_error("nothing selected — click a mark first, or drag a box round some.");
                        return;
                    }
                }
                self.arm(Tool::Modify(pick), page);
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
        self.tab_mut().pointer = Default::default();
        if self.paste_ghost.take().is_some() {
            self.say_info("nothing was pasted.");
        }
        if self.organize_open {
            self.organize_open = false;
            self.say_info("Organize closed.");
        }
        if !self.tab_mut().organize_selected.is_empty() || self.tab_mut().organize_drag.is_some() {
            self.tab_mut().organize_selected.clear();
            self.tab_mut().organize_anchor = None;
            self.tab_mut().organize_drag = None;
        }
        if let Some(what) = self.tab_mut().awaiting_password.take() {
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
        // **Distinguishes a real discard from a true no-op.** Both used to
        // say "left as it was.", identical to `put_down_page_editors`'s own
        // message — indistinguishable in the session log from an Escape that
        // had nothing to lose. When typed changes actually existed, say so,
        // so a reported "my edit disappeared" can be matched to an Escape in
        // the log rather than left as an open question.
        if let Some(edit) = self.tab_mut().editing_run.take() {
            if self.edit_has_changes(&edit) {
                // Words the engine refused are not simply dropped: they go to the
                // clipboard, and the line says so.
                if edit.refusal.is_some() {
                    self.offer_typed_text(&edit);
                    self.say_info("escaped — the edit the engine refused was discarded; what was typed is on the clipboard.");
                } else {
                    self.say_info("escaped — the typed changes were discarded.");
                }
            } else {
                self.say_info("left as it was.");
            }
        }
        if self.tab_mut().new_text_box.take().is_some() {
            self.say_info("nothing was added.");
        }
        if self.tab_mut().object_tool.take().is_some() {
            self.tab_mut().selected = None;
            self.tab_mut().grab = None;
            self.tab_mut().group = Vec::new();
            self.tab_mut().marquee = None;
            self.tab_mut().group_grab = None;
            self.say_info("object tool put down.");
        }
        if self.tab_mut().signature_selected.take().is_some() {
            self.tab_mut().signature_grab = None;
            self.say_info("signature deselected.");
        }
        if self.tab_mut().placed_image_selected.take().is_some() {
            self.tab_mut().placed_image_grab = None;
            self.say_info("picture deselected.");
        }
        if self.tab_mut().markup_armed.take().is_some() {
            self.say_info("tool put down.");
        }
        if std::mem::take(&mut self.tab_mut().link_armed) {
            self.say_info("tool put down.");
        }
        if std::mem::take(&mut self.tab_mut().match_properties_armed) {
            self.say_info("tool put down.");
        }
        if self.tab_mut().match_properties_sample.take().is_some() {
            self.say_info("tool put down.");
        }
        if let Some(reading) = &self.tab_mut().reading {
            // A real stop: the worker checks this between lines and puts the
            // page down.
            reading.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(armed) = self.tab_mut().tool.take() {
            self.say_info("cancelled.");
            if armed.kind.cancel_drops_checkpoint() {
                let page = self.tab().page;
                if let Some(layer) = self.tab_mut().markup.existing_mut(page) {
                    layer.forget_last_step();
                }
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
    /// Hide one rectangle — a dragged area, which is a list of one shape.
    fn lock_area(
        &mut self,
        page: usize,
        area: pdf_core::document::Rect,
        passcode: &[u8],
        require_complete: bool,
    ) -> Result<String, String> {
        self.lock_shapes(page, &[area], passcode, require_complete)
    }

    fn lock_shapes(
        &mut self,
        page: usize,
        shapes: &[pdf_core::document::Rect],
        passcode: &[u8],
        require_complete: bool,
    ) -> Result<String, String> {
        if passcode.is_empty() {
            return Err("a lock needs a passcode.".into());
        }
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            return Err("nothing open.".into());
        };
        let fonts = self.outlined_font_bytes();
        let font_refs: Vec<&[u8]> = fonts.iter().map(|f| f.as_slice()).collect();
        match session.lock_shapes(page, shapes, passcode, &font_refs, require_complete) {
            Ok(report) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.tab_mut().text_selection = None;
                self.tab_mut().find_hits.clear();
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
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else { return Ok(()) };
        for page in pages {
            let Some(layer) = self.tab_mut().markup.existing(*page) else { continue };
            session
                .commit_markup(*page, layer, MARKUP_INK, 1.5)
                .map_err(|e| format!("could not write the marks on page {}: {e}", page + 1))?;
        }
        // Written into the document now, so the layer must stop drawing them —
        // otherwise every mark appears twice.
        for page in pages {
            self.tab_mut().markup.forget(*page);
        }
        Ok(())
    }

    /// Whether a lock covers the whole of a page.
    ///
    /// Cheap to ask — the lock is cached — but asked only where a person is
    /// about to change something, never while drawing.
    fn page_is_locked(&self, page: usize) -> bool {
        let Some(doc) = &self.tab().doc else { return false };
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
        let doc = self.tab_mut().doc.as_mut().ok_or_else(|| "nothing open.".to_string())?;
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

        let doc = self.tab_mut().doc.as_mut().ok_or_else(|| "nothing open.".to_string())?;
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
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        match doc.session.lock_pages(pages, passcode) {
            Ok(locked) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.tab_mut().text_selection = None;
                self.tab_mut().find_hits.clear();
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
        self.tab().doc
            .as_ref()
            .and_then(|d| d.session.images_on(page).ok())
            .unwrap_or_default()
    }

    /// Put something at the front or the back of a page's drawing order.
    fn restack(
        &mut self,
        page: usize,
        object: usize,
        where_to: pdf_core::document::Stacking,
    ) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        // What a step will pass, asked before the page changes under it.
        let passing = match where_to {
            pdf_core::document::Stacking::Up => doc.session.stacking_neighbour(page, object, true).ok().flatten(),
            pdf_core::document::Stacking::Down => doc.session.stacking_neighbour(page, object, false).ok().flatten(),
            _ => None,
        };
        // What the picked layer looks like **now, before the page is
        // rewritten** — found again afterwards by what it looked like, not
        // by the index it happened to be at, which the restack itself moves.
        let follow = self.tab_mut().picked_layer.and_then(|at| self.layers_on(page).get(at).cloned());
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session.restack(page, object, where_to).map_err(|e| format!("layers: {e}"))?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        // The page has been rewritten, so the fresh list is searched for the
        // same thing by what it looked like — a one-step move is usually the
        // first of several, so the same thing is found again and stays picked.
        self.tab_mut().picked_layer = follow.as_ref().and_then(|was| {
            let close = |a: f32, b: f32| (a - b).abs() < 0.5;
            self.layers_on(page).iter().position(|d| {
                d.kind == was.kind
                    && d.depth == was.depth
                    && close(d.rect.left, was.rect.left)
                    && close(d.rect.top, was.rect.top)
                    && close(d.rect.right, was.rect.right)
                    && close(d.rect.bottom, was.rect.bottom)
            })
        });
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();

        // **Say when nothing will look different.** The order changed, and
        // that is real — but if nothing else is drawn where this thing is, the
        // page looks exactly as it did, and a reader pressing the button again
        // and again was reported as "it doesn't do anything".
        let overlaps = follow.as_ref().is_some_and(|me| {
            self.layers_on(page).iter().any(|d| {
                d.depth == 0
                    && !(d.kind == me.kind
                        && (d.rect.left - me.rect.left).abs() < 0.5
                        && (d.rect.top - me.rect.top).abs() < 0.5)
                    && d.rect.left < me.rect.right
                    && d.rect.right > me.rect.left
                    && d.rect.top < me.rect.bottom
                    && d.rect.bottom > me.rect.top
            })
        });
        let note = if overlaps {
            ""
        } else {
            " Nothing else is drawn where it is, so it looks the same — the order \
             matters once something overlaps it."
        };
        Ok(match where_to {
            pdf_core::document::Stacking::Front => {
                format!("brought to the front of page {}.{note}", page + 1)
            }
            pdf_core::document::Stacking::Back => {
                format!(
                    "sent to the back of page {} — behind everything it overlaps, in front \
                     of anything that would hide it.{note}",
                    page + 1
                )
            }
            pdf_core::document::Stacking::Up => match passing {
                Some(over) => format!(
                    "now in front of the {} {:?}.",
                    over.kind.describe(),
                    over.label.chars().take(30).collect::<String>()
                ),
                None => format!("moved up one on page {}.{note}", page + 1),
            },
            pdf_core::document::Stacking::Down => match passing {
                Some(under) => format!(
                    "now behind the {} {:?}.",
                    under.kind.describe(),
                    under.label.chars().take(30).collect::<String>()
                ),
                None => format!("moved down one on page {}.{note}", page + 1),
            },
        })
    }

    /// Everything drawn at a point, **topmost first**.
    ///
    /// **The whole stack, not the top of it.** Somebody right-clicking where
    /// their picture used to be is pointing at whatever is now covering it, so
    /// a menu that offered only the thing under the pointer would offer them
    /// the wrong object — and "bring to front" would raise the very panel they
    /// are trying to get out from behind.
    fn layers_under(&mut self, page: usize, at: AppPoint) -> Vec<usize> {
        let mut found: Vec<usize> = self
            .layers_on(page)
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                at.x >= d.rect.left as f64
                    && at.x <= d.rect.right as f64
                    && at.y >= d.rect.top as f64
                    && at.y <= d.rect.bottom as f64
            })
            .map(|(index, _)| index)
            .collect();
        // The list is in drawing order, so the last is on top.
        found.reverse();
        found
    }

    /// The row in the layer list for something found on the page, matched by
    /// object and — because a group's contents share the group's number — by
    /// where it sits.
    fn layer_index_for(
        &mut self,
        page: usize,
        object: usize,
        rect: pdf_core::document::Rect,
    ) -> Option<usize> {
        let close = |a: f32, b: f32| (a - b).abs() < 0.5;
        self.layers_on(page).iter().position(|d| {
            d.object == object
                && close(d.rect.left, rect.left)
                && close(d.rect.top, rect.top)
                && close(d.rect.right, rect.right)
                && close(d.rect.bottom, rect.bottom)
        })
    }

    /// Show the layer rail with one entry picked, by its place in the list.
    fn pick_layer(&mut self, page: usize, at: usize) {
        self.show_layers = true;
        self.tab_mut().picked_layer = Some(at);
        let told = self
            .layers_on(page)
            .get(at)
            .map(|d| format!("{} — {}", d.kind.describe(), d.label))
            .unwrap_or_default();
        self.say_info(format!("layers: {told}"));
    }

    /// Move whatever the layer rail has picked, or say why it cannot.
    fn restack_picked(&mut self, where_to: pdf_core::document::Stacking) {
        let page = self.tab_mut().page;
        let Some(at) = self.tab_mut().picked_layer else {
            self.say_info("pick something in the layer rail first — `layers` opens it.");
            return;
        };
        let Some(entry) = self.layers_on(page).get(at).cloned() else {
            self.say_error("that layer is no longer there.");
            return;
        };
        // Something inside a group moves *with* the group — which is what
        // somebody who picked a picture inside one means, and is why this acts
        // rather than declining.
        let grouped = !entry.movable;
        match self.restack(page, entry.object, where_to) {
            Ok(said) => self.say_info(if grouped {
                format!("{said} (the group it is drawn in went with it.)")
            } else {
                said
            }),
            Err(e) => self.say_error(e),
        }
    }

    /// What is sealed on a page, for drawing its padlocks.
    fn locked_items_on(&self, page: usize) -> Vec<pdf_core::document::LockedItem> {
        let Some(doc) = self.tab().doc.as_ref() else { return Vec::new() };
        // Kept — see [`caches::DocCaches::locked`]: this is asked every frame,
        // and a call into the engine waits for any render that is under way.
        if let Some(held) = doc.caches.locked.borrow().get(&page) {
            return held.clone();
        }
        let items = doc.session.locked_items_on(page);
        doc.caches.locked.borrow_mut().insert(page, items.clone());
        items
    }

    /// Lock the selected text, from the right-click menu.
    ///
    /// Goes through `lock_area` over the selection's own bounds — the words
    /// come off the page and the page as it was is sealed. **Not an item seal
    /// like an image gets**: a run's font is a document resource, and this
    /// module's `crypto::vault` explains at length why a text run restored on
    /// its own cannot be trusted to come back in the typeface it left in.
    fn lock_selection(&mut self) {
        let page = self.tab_mut().selection_page;
        let Some(range) = self.tab_mut().text_selection.clone() else {
            self.say_error("nothing selected.");
            return;
        };
        let Some(chars) = self.characters(page) else {
            self.say_error("nothing selected.");
            return;
        };
        // **One shape per line, and they travel that way.**
        //
        // The union of them is not the selection: a drag from the middle of one
        // line to the middle of the next makes a rectangle that also holds the
        // start of the first line and the end of the second, and locking that
        // takes words nobody picked. Measured on `two-column.pdf` before this:
        // a 39-character selection locked 68 characters and took the word
        // before it off the page.
        let shapes: Vec<pdf_core::document::Rect> = chars
            .line_rects(range)
            .into_iter()
            .map(|r| pdf_core::document::Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            })
            .collect();
        if shapes.is_empty() {
            self.say_error("nothing selected.");
            return;
        }

        // `require_complete: false` — the selection is the words, and an
        // image beneath them was never part of it.
        self.ask_or_reuse_passcode(
            Awaiting::Lock { page, shapes, require_complete: false },
            "type a passcode to lock the selection with, or Escape to give up.",
        );
    }

    /// Take one image off the page, sealed under a passcode.
    fn lock_image(&mut self, page: usize, object: usize, passcode: &[u8]) -> Result<String, String> {
        if passcode.is_empty() {
            return Err("a lock needs a passcode.".into());
        }
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
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
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        match doc.session.unlock_item(id, passcode) {
            Ok(()) => {
                self.after_locked_items_changed();
                Ok("unlocked.".into())
            }
            Err(e) => Err(format!("unlock: {e}")),
        }
    }

    /// Finish any lock whose badge stands over a picture that is still on the
    /// page, and say so. `quiet` keeps silent when there was nothing to do,
    /// which is every ordinary open.
    fn repair_locks(&mut self, say_if_nothing: bool) {
        let repaired = self.tab_mut().doc.as_ref().and_then(|d| d.session.repair_locks().ok());
        match repaired {
            Some((completed, dropped)) if (completed, dropped) != (0, 0) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                let mut said = Vec::new();
                if completed > 0 {
                    said.push(format!(
                        "{completed} picture lock{} that had never taken effect {} now been \
                         completed — the passcode still brings {} back",
                        if completed == 1 { "" } else { "s" },
                        if completed == 1 { "has" } else { "have" },
                        if completed == 1 { "it" } else { "them" },
                    ));
                }
                if dropped > 0 {
                    said.push(format!(
                        "{dropped} lock badge{} stood over a picture that could not be taken \
                         off the page and {} been removed",
                        if dropped == 1 { "" } else { "s" },
                        if dropped == 1 { "has" } else { "have" },
                    ));
                }
                self.say_info(format!("{}. Save to keep this.", said.join("; ")));
            }
            Some(_) if say_if_nothing => self.say_info("every lock in this document is whole."),
            _ => {}
        }
    }

    /// What every lock and unlock of an item has to invalidate.
    ///
    /// The page has been rewritten, so the rendered tiles and anything holding
    /// its text are stale — and the badges themselves are re-read rather than
    /// cached, so nothing else has to be told.
    fn after_locked_items_changed(&mut self) {
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        self.tab_mut().selected_image = None;
    }

    /// Bring back everything the passcode has sealed.
    ///
    /// Decryption happens here; the pages go back **through the command stack**,
    /// so the restore undoes like any other edit and no passcode goes near it.
    fn unlock(&mut self, passcode: &[u8]) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
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

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
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
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            return Err("nothing open.".into());
        };

        // The same candidate faces `apply_redaction` below will actually use —
        // the preview has to agree with what applying would do, or it could
        // promise a clean redaction the apply step then cannot deliver.
        let fonts = self.outlined_font_bytes();
        let font_refs: Vec<&[u8]> = fonts.iter().map(|f| f.as_slice()).collect();
        let report = session
            .preview_redaction(page, area, &font_refs)
            .map_err(|e| format!("redact: {e}"))?;

        if !report.blockers().is_empty() {
            self.tab_mut().asking_to_redact = Some(PendingRedaction { page, area, report });
            // Not an error — the question is on screen.
            return Ok(String::new());
        }
        let notes = shared_form_notes(&report);
        self.apply_redaction(page, area, false).map(|said| said + &notes)
    }

    /// Run the redaction for real, through the command stack so it undoes.
    fn apply_redaction(
        &mut self,
        page: usize,
        area: pdf_core::document::Rect,
        allow_incomplete: bool,
    ) -> Result<String, String> {
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            return Err("nothing open.".into());
        };
        // Copied into the command so redo re-executes against the same faces
        // later, not against whatever this build happens to bundle — or a
        // reader happens to have added — by then.
        let outlined_fonts = self.outlined_font_bytes();
        match session.execute(pdf_core::command::Command::Redact {
            page_index: page,
            area,
            fill: Some(pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 }),
            allow_incomplete,
            outlined_fonts,
        }) {
            Ok(_) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                // The words are gone, so anything holding on to them is stale.
                self.tab_mut().text_selection = None;
                self.tab_mut().find_hits.clear();
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
        let Some(asking) = self.tab_mut().asking_to_redact.clone() else { return };
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
                self.tab_mut().asking_to_redact = None;
                self.say_info("nothing was redacted.");
            }
            Some(true) => {
                self.tab_mut().asking_to_redact = None;
                let notes = shared_form_notes(&asking.report);
                match self.apply_redaction(asking.page, asking.area, true) {
                    Ok(said) => self.say_info(said + &notes),
                    Err(e) => self.say_error(e),
                }
            }
        }
    }

    /// Put down whatever the page itself was mid-way through — an open run
    /// editor, or a text box being composed — the same way Escape already
    /// does. Every place a *different* tool gets taken up calls this first.
    ///
    /// **Reported from use: text picked with Edit Text, still open, got its
    /// run split into individual characters the moment Edit Object was
    /// clicked without an Escape first.** Edit Object's own click-to-select
    /// does that split deliberately — see `split_if_whole_run` — but nothing
    /// had told it a different tool was already holding that exact run
    /// open, so the very next click, meant for Edit Object, silently
    /// fragmented the run Edit Text still thought it was editing. Once
    /// split, Edit Text can never tell that piece apart from any other tiny
    /// run again — every future click on it lands on a single character.
    fn put_down_page_editors(&mut self, reason: &str) {
        // **Same distinction as `escape`'s own** — see its comment. `reason`
        // names the action that put the editor down (arming a different
        // tool, picking up Edit Object, starting a new text box), so a
        // discard-with-changes shows up in the session log as exactly which
        // of those it was, not just that *something* closed the editor.
        if let Some(edit) = self.tab_mut().editing_run.take() {
            if self.edit_has_changes(&edit) {
                if edit.refusal.is_some() {
                    self.offer_typed_text(&edit);
                    self.say_info(format!(
                        "{reason} — the edit the engine refused was discarded; what was typed is on the clipboard."
                    ));
                } else {
                    self.say_info(format!("{reason} — the unapplied edit was discarded."));
                }
            } else {
                self.say_info("left as it was.");
            }
        }
        if self.tab_mut().new_text_box.take().is_some() {
            self.say_info("nothing was added.");
        }
        if self.tab_mut().pending_link.take().is_some() {
            self.say_info("nothing was linked.");
        }
        if self.tab_mut().pending_article_box.take().is_some() {
            self.say_info("nothing was added.");
        }
    }




    /// Poll both of the two separate undo stacks' own monotonic edit
    /// counters and note which one has moved since the last time this ran —
    /// called once a frame, well before anyone can press `undo`.
    ///
    /// **Reported from use: a shape drawn earlier got undone instead of a
    /// text box just added a moment ago.** `undo_redo` always tried the
    /// current page's markup layer first, regardless of which of it and the
    /// document's own command history had actually changed more recently —
    /// harmless on a page whose layer was never touched, wrong the moment a
    /// page carried both a drawn shape and an edited or newly written piece
    /// of text, since the layer would win every time whether or not it was
    /// the more recent of the two.
    fn track_undo_recency(&mut self) {
        let page = self.tab().page;
        let layer_edits = self.tab().markup.existing(page).map(|l| l.edits()).unwrap_or(0);
        if layer_edits != self.tab_mut().last_layer_edits {
            self.tab_mut().last_layer_edits = layer_edits;
            self.tab_mut().prefer_layer_undo = true;
        }
        let doc_generation = self.tab_mut().doc.as_ref().map(|d| d.session.undo_generation()).unwrap_or(0);
        if doc_generation != self.tab_mut().last_doc_generation {
            self.tab_mut().last_doc_generation = doc_generation;
            self.tab_mut().prefer_layer_undo = false;
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        self.track_undo_recency();
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let tried = if self.tab_mut().prefer_layer_undo {
            self.try_layer_undo_redo(undo) || self.try_doc_undo_redo(undo)
        } else {
            self.try_doc_undo_redo(undo) || self.try_layer_undo_redo(undo)
        };
        if !tried {
            self.say_info(if undo {
                "nothing to undo — neither the marks on this page nor the document."
            } else {
                "nothing to redo."
            });
        }
    }

    /// Step the current page's markup layer back or forward one checkpoint.
    /// `false` means there was nothing there to step — not an error, just
    /// this stack's turn to defer to [`Self::try_doc_undo_redo`].
    fn try_layer_undo_redo(&mut self, undo: bool) -> bool {
        let page = self.tab_mut().page;
        let Some(layer) = self.tab_mut().markup.existing_mut(page) else { return false };
        let stepped = if undo { layer.undo() } else { layer.redo() };
        match stepped {
            Some(what) => {
                self.say_info(format!("{} {what}.", if undo { "undid" } else { "redid" }));
                true
            }
            None => false,
        }
    }

    /// The document's own command history — the other of the two stacks
    /// [`Self::undo_redo`] juggles. An outright failure is reported and
    /// still counts as handled: falling through to the layer after an error
    /// would make one press of undo report two different things.
    fn try_doc_undo_redo(&mut self, undo: bool) -> bool {
        let Some(doc) = &self.tab_mut().doc else { return false };
        let outcome = if undo { doc.session.undo() } else { doc.session.redo() };
        match outcome {
            Ok((true, state)) => {
                let label = if undo { state.redo_label.clone() } else { state.undo_label.clone() };
                self.say_info(label.unwrap_or_else(|| "done.".into()));
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                // What a page draws can have changed shape entirely — an
                // object put back, one taken away again, a run re-merged
                // from its split characters — so the cached list this feeds
                // the layers rail and every hit-test from is exactly as
                // stale as the raster this already knew to drop.
                // The same staleness `apply_editing_page` already clears
                // after a successful Apply — undoing (or redoing) that same
                // edit changes exactly the same words, and a search or a
                // selection built from the pre-undo text would otherwise
                // keep answering as if the edit were still there.
                self.tab_mut().text_selection = None;
                self.tab_mut().find_hits.clear();
                // Undoing or redoing an `AddBookmark` is the one document
                // change with nothing else here to notice it by — no page
                // raster changes, no object list to compare — so the
                // corner icon's own cache is refreshed unconditionally
                // rather than only when something else already knew to.
                self.sync_bookmarks();
                true
            }
            Ok((false, _)) => false,
            Err(e) => {
                self.say_error(explain(&e));
                true
            }
        }
    }

    /// Ask for a passcode to lock something with — or use the one already in
    /// force, if this document has locks and it has been typed once.
    ///
    /// **Every locking gesture goes through here.** The three of them —
    /// selected words, a dragged area, an image, whole pages — used to arm
    /// `awaiting_password` each in their own way, which is how asking twice for
    /// the same secret became four separate places to fix.
    fn ask_or_reuse_passcode(&mut self, what: Awaiting, prompt: &str) {
        if let Some(held) = self.tab_mut().held_passcode.clone() {
            // Straight through, by the same route the window takes, so a held
            // passcode and a typed one cannot drift apart.
            self.tab_mut().awaiting_password = Some(what);
            self.answer_lock_passcode(&held);
            return;
        }
        self.tab_mut().awaiting_password = Some(what);
        self.say_info(prompt);
    }

    /// Let go of the passcode being held for this document.
    ///
    /// Called whenever the document does — closing one and opening another must
    /// not carry a secret across, and a document that is no longer on screen has
    /// no business leaving one in memory.
    fn forget_passcode(&mut self) {
        self.tab_mut().held_passcode = None;
    }

    /// Hand a passcode to whichever lock is waiting for it.
    ///
    /// Its own method so the window and a test take one path — the alternative
    /// is a dialog nothing exercises.
    fn answer_lock_passcode(&mut self, typed: &str) {
        let Some(what) = self.tab_mut().awaiting_password.take() else { return };
        match what {
            Awaiting::Lock { page, shapes, require_complete } => {
                let done = self.lock_shapes(page, &shapes, typed.as_bytes(), require_complete);
                self.hold_or_drop(typed, &done);
                match done {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Awaiting::LockPages(pages) => {
                let done = self.lock_pages(&pages, typed.as_bytes());
                self.hold_or_drop(typed, &done);
                match done {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Awaiting::LockImage { page, object } => {
                let done = self.lock_image(page, object, typed.as_bytes());
                self.hold_or_drop(typed, &done);
                match done {
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
            other => self.tab_mut().awaiting_password = Some(other),
        }
    }

    /// Keep a passcode that locked something; let go of one that did not.
    ///
    /// **Both halves matter.** Keeping it is the feature. Dropping it on
    /// failure is what stops a held passcode that has stopped working — a
    /// document saved under a new one, say — from silently failing every lock
    /// after it while the person watching is never given the chance to type the
    /// right one.
    fn hold_or_drop(&mut self, typed: &str, outcome: &Result<String, String>) {
        match outcome {
            Ok(_) => self.tab_mut().held_passcode = Some(zeroize::Zeroizing::new(typed.to_owned())),
            Err(_) => self.forget_passcode(),
        }
    }

    /// Whether this document already has a lock in it.
    ///
    /// Decides whether a passcode is being **chosen** — new, so it must be
    /// strong and typed twice — or **used**, which is neither. A document
    /// locked before this rule existed has to stay lockable.
    fn lock_exists(&self) -> bool {
        self.tab().doc.as_ref().is_some_and(|d| !d.session.locked_pages().is_empty())
    }





    /// Act on a password somebody has typed, whatever it was asked for.
    ///
    /// **The decision, separated from the window.** Whether a password is
    /// strong enough, whether it needs typing again, and what it finally does
    /// are all decided here — so a test can exercise them without rendering,
    /// and the window cannot drift from what those tests check.
    fn answer_passcode(&mut self, typed: &str) {
        use pagify_shell::passphrase;

        let Some(waiting) = self.tab_mut().awaiting_password.clone() else { return };
        self.tab_mut().password_field_focused = false;
        self.tab_mut().password_problem = None;

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
                self.tab_mut().password_problem = Some(problem);
                return;
            }
            let Some(then) = self.tab_mut().awaiting_password.take() else { return };
            self.tab_mut().awaiting_password = Some(match then {
                Awaiting::Secure(options) => {
                    Awaiting::SecureAgain { first: typed.to_string(), options }
                }
                other => Awaiting::LockAgain { first: typed.to_string(), then: Box::new(other) },
            });
            return;
        }

        match self.tab_mut().awaiting_password.clone() {
            Some(Awaiting::LockAgain { first, then }) => {
                if typed != first {
                    self.tab_mut().awaiting_password = None;
                    self.say_error("those did not match — nothing was locked. Try again.");
                    return;
                }
                self.tab_mut().awaiting_password = Some(*then);
                self.answer_lock_passcode(typed);
            }
            Some(Awaiting::SecureAgain { first, options }) => {
                if typed != first {
                    self.tab_mut().awaiting_password = None;
                    self.say_error("those did not match — nothing was set. Run `secure` again.");
                    return;
                }
                self.tab_mut().awaiting_password = None;
                let outcome = if self.tab_mut().password_plus {
                    // The window refuses this before a password is typed; the
                    // same rule here, for the paths that do not go through it.
                    if options != pagify_shell::verbs::SecureOptions::default() {
                        Err(format!(
                            "Secure Plus keeps no permissions, so \"{}\" cannot be set under \
                             it — nothing was set. Use `secure` without Secure Plus for that.",
                            options.describe()
                        ))
                    } else {
                        self.secure_document_plus(typed.as_bytes())
                    }
                } else {
                    self.secure_document(typed.as_bytes(), options)
                };
                match outcome {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
            }
            Some(Awaiting::SecureCurrent(options)) => {
                if !self.tab_mut().doc.as_ref().is_some_and(|d| d.session.password_matches(typed.as_bytes()))
                {
                    self.tab_mut().password_problem = Some("That is not this document's password.".into());
                    return;
                }
                // Right, so the old one comes off and a new one is chosen.
                if let Some(doc) = &self.tab_mut().doc {
                    if let Err(e) = doc.session.unsecure_document() {
                        self.tab_mut().awaiting_password = None;
                        self.say_error(e.to_string());
                        return;
                    }
                }
                self.tab_mut().awaiting_password = Some(Awaiting::Secure(options));
            }
            Some(Awaiting::Certificate(path)) => {
                self.tab_mut().awaiting_password = None;
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
        self.tab_mut().awaiting_password = None;
        self.tab_mut().password_field_focused = false;
        self.open_with(path, Some(typed));
        // Still asking means it was not accepted, and the window says so rather
        // than the status line underneath it.
        self.tab_mut().password_problem = matches!(self.tab_mut().awaiting_password, Some(Awaiting::Open(_)))
            .then(|| "That password was not accepted.".to_string());
    }

    /// The path a document is waiting on a password for, if any.
    fn awaiting_open(&self) -> Option<String> {
        match &self.tab().awaiting_password {
            Some(Awaiting::Open(path)) => Some(path.clone()),
            _ => None,
        }
    }

    fn save(&mut self, dest: Option<PathBuf>) -> SaveOutcome {
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return SaveOutcome::Failed;
        };
        let over_the_original = dest.is_none();
        let path = dest.unwrap_or_else(|| session.path().to_path_buf());

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
            && session.is_secured()
            && !session.had_password_on_open()
            && !std::mem::take(&mut self.tab_mut().secure_in_place_confirmed)
        {
            // Asked, not refused — see `asking_to_secure`.
            self.tab_mut().asking_to_secure = Some(path);
            self.say_info(
                "this save would put a password on the file itself — choose what to do \
                 in the window.",
            );
            return SaveOutcome::Asking;
        }

        // Commit every marked page before writing: real ink for any reader,
        // plus the live geometry so it is still editable next time.
        let mut stored = 0;
        let mut skipped = Vec::new();
        let marked_pages = self.tab().markup.marked_pages();
        for page in marked_pages {
            if let Some(layer) = self.tab_mut().markup.existing(page) {
                match session.commit_markup(page, layer, MARKUP_INK, 1.5) {
                    Ok(committed) => {
                        stored += committed.objects_stored;
                        skipped.extend(committed.skipped.into_iter().map(|s| s.what));
                    }
                    Err(e) => {
                        self.say_error(format!("could not write the markup on page {}: {e}", page + 1));
                        return SaveOutcome::Failed;
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
        let incremental = !session.must_save_full_copy();
        // A rewrite relocates every object, and a signature covers a byte
        // range: one made before the redaction or the password is void
        // afterwards. Said at the save, which is where it happens.
        let signatures_lost = if incremental { 0 } else { session.signature_count() };
        match session.save_to(&path, incremental) {
            Ok(()) => {
                self.tab_mut().saved_revision = self.tab_mut().markup.revision();
                self.say_info(format!(
                    "saved {} ({stored} marks){}.",
                    path.display(),
                    if incremental { "" } else { " — the file was rewritten" }
                ));
                if signatures_lost > 0 {
                    self.say_error(format!(
                        "the rewrite broke {signatures_lost} signature{} — sign after redacting \
                         or securing, not before.",
                        if signatures_lost == 1 { "" } else { "s" }
                    ));
                }
                if !skipped.is_empty() {
                    self.say_error(format!(
                        "{} mark{} could not be stored and will not come back: {}",
                        skipped.len(),
                        if skipped.len() == 1 { "" } else { "s" },
                        skipped.join(", ")
                    ));
                }
                SaveOutcome::Done
            }
            Err(e) => {
                self.say_error(format!("save failed: {e}"));
                SaveOutcome::Failed
            }
        }
    }

    /// The window that asks what a save should do about a password that has
    /// not been written yet — see `asking_to_secure`.
    fn ask_about_securing(&mut self, ctx: &egui::Context) {
        let Some(original) = self.tab_mut().asking_to_secure.clone() else { return };
        let name = original
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "this file".to_string());

        #[derive(Clone, Copy)]
        enum Choice {
            SaveCopy,
            DropPassword,
            WriteOver,
            Cancel,
        }
        let mut choice: Option<Choice> = None;
        egui::Modal::new(egui::Id::new("securing")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Put a password on this file?");
            ui.add_space(6.0);
            ui.label(format!(
                "{name} is not password-protected on disk. Saving now writes the \
                 password you chose over it: anyone opening the file — including you — \
                 will need that password, and there is no way back if it is forgotten."
            ));
            ui.add_space(4.0);
            ui.small("Locked text and pictures keep their own passcode either way.");
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save a secured copy as…").clicked() {
                    choice = Some(Choice::SaveCopy);
                }
                if ui.button("Take the password off and save").clicked() {
                    choice = Some(Choice::DropPassword);
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Write the password over the original").clicked() {
                    choice = Some(Choice::WriteOver);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(Choice::Cancel);
        }

        let Some(choice) = choice else { return };
        self.tab_mut().asking_to_secure = None;
        match choice {
            Choice::Cancel => self.say_info("not saved."),
            Choice::SaveCopy => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Save a secured copy")
                    .add_filter("PDF", &["pdf"]);
                if let Some(dir) = original.parent() {
                    dialog = dialog.set_directory(dir);
                }
                let suggested = original
                    .file_stem()
                    .map(|s| format!("{} (secured).pdf", s.to_string_lossy()))
                    .unwrap_or_else(|| "secured.pdf".to_string());
                match dialog.set_file_name(&suggested).save_file() {
                    Some(dest) => {
                        self.save(Some(dest));
                    }
                    None => self.say_info("not saved."),
                }
            }
            Choice::DropPassword => {
                let dropped = self.tab_mut().doc.as_ref().map(|d| d.session.unsecure_document());
                match dropped {
                    Some(Ok(())) => {
                        self.say_info("password taken off.");
                        self.save(None);
                    }
                    Some(Err(e)) => self.say_error(format!("could not take the password off: {e}")),
                    None => {}
                }
            }
            Choice::WriteOver => {
                self.tab_mut().secure_in_place_confirmed = true;
                self.save(None);
            }
        }
    }

    // -- organize -----------------------------------------------------------

    fn with_pages<F>(&mut self, spec: &str, what: &str, mut f: F)
    where
        F: FnMut(&Session, Vec<usize>) -> Result<String, String>,
    {
        let Some(doc) = &self.tab_mut().doc else {
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
        let Some(doc) = &mut self.tab_mut().doc else { return };
        doc.rendered_is_stale();
        if let (Ok(count), Ok(sizes)) = (doc.session.page_count(), doc.session.page_sizes()) {
            doc.page_count = count;
            // Rebuilt in the layout the reader chose. `Strip::new` is
            // single-page, and using it here would quietly put a spread back to
            // one-up every time a page was added or removed — and the turn the
            // view is at is kept for the same reason.
            doc.strip = Strip::with_layout_turned(&sizes, PAGE_GAP_PT, doc.strip.layout(), doc.strip.turned());
            self.tab_mut().page = self.tab_mut().page.min(count.saturating_sub(1));
            self.tab_mut().zoom_basis = self.tab_mut().zoom_basis.min(count.saturating_sub(1));
        }
    }

    /// Whether `path` is the file this tab has open — the one place an extract
    /// or a copy must never be written, because the document is being read from
    /// it.
    fn is_open_document(&self, path: &std::path::Path) -> bool {
        let Some(doc) = &self.tab().doc else { return false };
        let open = doc.session.path();
        if path == open {
            return true;
        }
        match (std::fs::canonicalize(path), std::fs::canonicalize(open)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// Open the Extract dialog on this tab, asking for the selected pages when
    /// the Organize grid or the rail has some, otherwise the one on screen.
    fn open_extract_dialog(&mut self) {
        if self.tab().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let pages = if self.tab().organize_selected.is_empty() {
            (self.tab().page + 1).to_string()
        } else {
            compact_page_spec(&self.tab().organize_selected)
        };
        self.tab_mut().extract_ask = Some(ExtractAsk { pages, ..Default::default() });
    }

    /// Where to write the extracted pages: the system's Save box, started in the
    /// document's own folder with a name that says which pages these are.
    fn pick_extract_destination(&self, pages: &[usize]) -> Option<std::path::PathBuf> {
        let current = self.tab().doc.as_ref()?.session.path().to_path_buf();
        let stem = current
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".into());
        let spec = compact_page_spec(pages);
        let name = if spec.len() <= 40 {
            format!("{stem} (pages {spec}).pdf")
        } else {
            format!("{stem} (extract).pdf")
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Extract pages to")
            .add_filter("PDF", &["pdf"])
            .set_file_name(name);
        if let Some(dir) = current.parent() {
            dialog = dialog.set_directory(dir);
        }
        let mut path = dialog.save_file()?;
        if path.extension().is_none() {
            path.set_extension("pdf");
        }
        Some(path)
    }


    fn extract(&mut self, spec: &str, dest: &std::path::Path) {
        // Refused here, not only in the dialog: the typed `extract 1-3 <path>`
        // reaches this too, and writing over the open document would replace the
        // file the session is reading from.
        if self.is_open_document(dest) {
            self.say_error("extract: that is the document you have open — name another file.");
            return;
        }
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
        let Some(doc) = &self.tab_mut().doc else {
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
        let Some(doc) = &self.tab_mut().doc else {
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
        let Some(doc) = &self.tab_mut().doc else {
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
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return;
        };
        let page_count = self.tab().doc.as_ref().map(|d| d.page_count).unwrap_or(0);
        let pages = if spec.trim().is_empty() {
            vec![self.tab().page]
        } else {
            match pagify_shell::organize::parse_range(spec, page_count) {
                Ok(pages) => pages,
                Err(why) => {
                    self.say_error(why);
                    return;
                }
            }
        };

        let after = pages.iter().copied().max().map(|p| p + 1).unwrap_or(0);
        let count = pages.len();
        match session.duplicate_pages(&pages, after) {
            Ok(_) => self.after_page_change(format!(
                "{count} page{} duplicated.",
                if count == 1 { "" } else { "s" }
            )),
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Change a word that is already on the page — click on it, retype it.
    fn edit_text(&mut self) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let page = self.tab_mut().page;
        self.arm(Tool::PickText, page);
    }

    /// The run of text under a point, offered for retyping — and, when the
    /// click is in a paragraph, the whole paragraph it belongs to.
    ///
    /// **One line per click goes to the session log** (kind `pick`, see
    /// [`block_input::format_pick_line`]): what was clicked, which way it went
    /// and why, how big the paragraph was and why it stopped, and what it cost.
    /// This is only the wrapper that writes it, so that *every* way out of
    /// [`Self::pick_text_run_traced`] — a paragraph, one run, a joined group, a
    /// drawn word, a refusal, nothing there — is logged the same way.
    fn pick_text_run(&mut self, page: usize, at: AppPoint) -> Result<String, String> {
        let started = std::time::Instant::now();
        let mut trace = PickTrace::new(page, (at.x as f32, at.y as f32));
        let outcome = self.pick_text_run_traced(page, at, &mut trace);
        trace.total_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.session_log.record("pick", &block_input::format_pick_line(&trace));
        outcome
    }

    /// The run under a click on a **page too heavy to read for paragraphs**: the
    /// words of every text object are read in one pass (once, see
    /// [`Self::light_page`]) and the click is resolved against them — the same
    /// smallest-rect-then-nearest rule as every other click
    /// ([`block_input::pick_seed`]). If the one pass cannot be made the way of the
    /// last resort ([`Self::legacy_run_under`]) is tried.
    fn heavy_run_under(
        &self,
        page: usize,
        x: f32,
        y: f32,
    ) -> Result<Option<(pdf_core::document::TextRun, block_input::SeedRule)>, String> {
        let Ok(light) = self.light_page(page, true) else { return self.legacy_run_under(page, x, y) };
        let Some((seed, rule)) = block_input::pick_seed(&light, x, y, HIT_TOLERANCE_PT as f32) else {
            return Ok(None);
        };
        Ok(light.runs.get(&seed).cloned().map(|run| (run, rule)))
    }

    /// The run under a click when the page's text could not be read in one
    /// pass: the pre-detector way, kept so that Edit Text never stops working
    /// altogether. Rectangles first and words for the one run only, as the
    /// tool always did; no paragraph, no fonts or shapes.
    ///
    /// The seed rule is [`block_input::pick_seed`]'s, on a page of rectangles
    /// with no words: the same smallest-rect-then-nearest rule, written once.
    fn legacy_run_under(
        &self,
        page: usize,
        x: f32,
        y: f32,
    ) -> Result<Option<(pdf_core::document::TextRun, block_input::SeedRule)>, String> {
        let Some(doc) = self.tab().doc.as_ref() else { return Err("nothing open.".into()) };
        let page_of_rects = self.light_page(page, false)?;
        let Some((seed, rule)) = block_input::pick_seed(&page_of_rects, x, y, HIT_TOLERANCE_PT as f32) else {
            return Ok(None);
        };
        // A page read the heavy way already has the words.
        if let Some(run) = page_of_rects.runs.get(&seed).filter(|run| !run.text.is_empty()) {
            return Ok(Some((run.clone(), rule)));
        }
        let wanted: std::collections::HashSet<usize> = std::iter::once(seed).collect();
        let runs = doc.session.text_runs_some(page, &wanted).map_err(|e| format!("{e}"))?;
        Ok(runs.into_iter().find(|r| r.object == seed).map(|run| (run, rule)))
    }

    /// A click that landed on no text object: a word the page *draws* as
    /// outlines, or the reason there is nothing to pick.
    fn no_text_here(&mut self, page: usize, at: AppPoint, trace: &mut PickTrace) -> Result<String, String> {
        // **A page of that many objects is not searched for drawn words**:
        // recognising them walks and matches every shape on it — seconds on a
        // drawing of tens of thousands, tens of seconds on a plan of nearly a
        // million — and the answer to a click on blank paper is not worth a frozen
        // window. Said, so the silence is not mistaken for there being none.
        if let Some(weight) = self.page_weight(page).filter(|w| w.page_objects > HEAVY_PAGE_OBJECTS) {
            return Err(format!(
                "no text there — click on some words. (This page holds {} objects: too many for words drawn as \
                 shapes to be looked for.)",
                weight.page_objects
            ));
        }
        // Nothing written here — but the page may *draw* words, and one of
        // those is still something a person can point at and mean.
        if let Some(word) = self.drawn_word_at(page, at) {
            trace.path = PickPath::Drawn;
            return Ok(word);
        }
        // A page whose words are drawn says so, and says what would let
        // them be read — rather than "no text", which is true of the file
        // and useless to the person looking at the words.
        let kind = self.tab_mut().doc.as_ref().and_then(|d| d.session.classify(page).ok()).map(|c| c.kind);
        let drawn = matches!(
            kind,
            Some(pdf_core::document::PageTextKind::Outlined | pdf_core::document::PageTextKind::Hybrid)
        );
        // **Only when the click was on the drawn words.** Reported from use,
        // in the logs: this was the answer to *every* blank click on a page
        // classified `Outlined` or `Hybrid` — and a page that is partly a
        // photograph is `Hybrid` — so bare paper, a margin, or the photo
        // itself was told to go and find a typeface. Twenty-five times,
        // none of them near a drawn word. A blank spot is a blank spot.
        if drawn && self.click_is_on_drawn_type(page, at) {
            return Err(
                "no text there — the words here look drawn rather than written, and \
                 reading them needs the typeface they were set in. \
                 `outlinedfont add <file.ttf>` supplies it."
                    .into(),
            );
        }
        // **A scan has no text to click on, and says so** rather than answering as
        // if the click had missed some: the page is a picture of words, and what
        // reads a picture of words is `extracttext`, which recognises them.
        if kind == Some(pdf_core::document::PageTextKind::Scanned) {
            return Err(
                "no text on this page — it looks scanned; `extracttext` reads it with OCR."
                    .into(),
            );
        }
        Err("no text there — click on some words.".into())
    }

    /// Open the paragraph the detector found as block `block` of `pb`.
    ///
    /// **The editor's lines are the detector's lines, one to one**, never
    /// regrouped: re-deriving rows from how the boxes overlap can split or
    /// merge them, and the editor's buffer has exactly one line per entry of
    /// [`EditingRun::lines`] — apply writes each typed line to its own line's
    /// objects. The block first has to pass
    /// [`block_input::check_editor_invariants`]; the `Err` says which
    /// invariant it broke, and the caller opens the one run instead.
    fn open_block(&mut self, page: usize, pb: &PageBlocks, block: usize) -> Result<String, String> {
        block_input::check_editor_invariants(pb, block)?;
        let specs = block_input::editor_lines(pb, block);
        // A line made only of outlined words has no text object, so no box of
        // its own to show a left edge from; it is given the paragraph's, so one
        // such line does not stop `paragraph_should_justify` from seeing a
        // paragraph whose every other line starts at the same margin. Only
        // ever moved outwards: the box has to keep covering the drawn words.
        let margin = specs
            .iter()
            .filter(|spec| !spec.objects.is_empty())
            .map(|spec| spec.rect.left)
            .fold(f32::INFINITY, f32::min);
        // Each line's twins (the faux bold of a heading) ride along with it,
        // so that retyping the line takes them off the page too.
        let twins: Vec<Vec<usize>> = specs.iter().map(|spec| spec.twins.clone()).collect();
        let lines: Vec<ParagraphLine> = specs
            .into_iter()
            .map(|mut spec| {
                if spec.objects.is_empty() && margin.is_finite() {
                    spec.rect.left = spec.rect.left.min(margin);
                }
                ParagraphLine { objects: spec.objects, rect: spec.rect, frozen: spec.frozen }
            })
            .collect();
        let runs: Vec<pdf_core::document::TextRun> =
            lines.iter().flat_map(|line| &line.objects).filter_map(|o| pb.runs.get(o).cloned()).collect();
        self.pick_paragraph(page, lines, twins, &runs, &pb.faces, &pb.styles, &pb.shapes)
    }

    /// Open an editor over `lines` — a paragraph already decided, whether by
    /// the detector ([`Self::open_block`]) or by a person
    /// ([`Self::open_joined_editor`]) — every line's own text joined with
    /// `\n`, top to bottom, so retyping means retyping the paragraph rather
    /// than only the one line that happened to be clicked. `runs` holds (at
    /// least) every object of every line, with `face_names`, `styles` and
    /// `shapes` from the same read of the page.
    fn pick_paragraph(
        &mut self,
        page: usize,
        lines: Vec<ParagraphLine>,
        twins: Vec<Vec<usize>>,
        runs: &[pdf_core::document::TextRun],
        face_names: &std::collections::HashMap<usize, String>,
        styles: &std::collections::HashMap<usize, pdf_core::document::RunStyle>,
        shapes: &[pdf_core::document::DrawnObject],
    ) -> Result<String, String> {
        let frozen_lines = lines.iter().filter(|line| line.frozen).count();
        let page_raster = self.page_raster_for_sampling(page);
        let (mut edit, look_object) =
            self.build_editor_from_lines(page, lines, runs, face_names, styles, shapes, page_raster.as_deref())?;
        // A paragraph with no twins carries none at all, not a list of empty
        // ones: the lists are a list per line or nothing (`check_lines_up`).
        if twins.iter().any(|line| !line.is_empty()) {
            edit.twins = twins;
        }
        let line_count = edit.lines.len();
        // Lets a future "it disappeared" report be checked against what the
        // document's text actually was at the moment it was picked.
        let picked_char_count = edit.buffer.chars().count();
        // The same face request a single run's own pick makes — see
        // `pick_text_run_traced`'s matching call — but from whichever look won
        // the tally, not always the first line's.
        self.want_document_face_for(page, look_object, styles.get(&look_object).map(|style| style.font));
        self.tab_mut().editing_run = Some(edit);

        // **Said at the pick, not discovered at the apply**: a line the page
        // draws as shapes can be read and retyped here but is never written.
        let drawn = match frozen_lines {
            0 => String::new(),
            1 => "; 1 line holds words drawn as shapes and is left exactly as it is".to_string(),
            n => format!("; {n} lines hold words drawn as shapes and are left exactly as they are"),
        };
        Ok(format!(
            "editing a paragraph of {} ({}){drawn} — Apply to keep, Escape to leave it.",
            count_of(line_count, "line", "lines"),
            count_of(picked_char_count, "character", "characters"),
        ))
    }

    /// Group runs a person picked out by hand into visual rows — there is no
    /// detector line structure for an arbitrary set of objects. `paragraph`
    /// is top to bottom; a run of consecutive selections that vertically
    /// overlap is one visual line a producer split across more than one run,
    /// sorted by x within the row, since two runs sharing a line are not
    /// guaranteed to arrive in reading order, only their vertical position is.
    /// Nothing here is frozen: it is picked from text alone.
    fn rows_from_selected(paragraph: Vec<Selected>) -> Vec<ParagraphLine> {
        let overlaps_v = |a: &pdf_core::document::Rect, b: &pdf_core::document::Rect| {
            a.top.min(a.bottom) < b.top.max(b.bottom) && b.top.min(b.bottom) < a.top.max(a.bottom)
        };
        let mut rows: Vec<Vec<Selected>> = Vec::new();
        for sel in paragraph {
            let joins_last = rows
                .last()
                .is_some_and(|row: &Vec<Selected>| row.iter().any(|s| overlaps_v(&s.rect, &sel.rect)));
            if joins_last {
                rows.last_mut().expect("checked").push(sel);
            } else {
                rows.push(vec![sel]);
            }
        }
        for row in &mut rows {
            row.sort_by(|a, b| a.rect.left.min(a.rect.right).total_cmp(&b.rect.left.min(b.rect.right)));
        }
        rows.iter()
            .map(|row| ParagraphLine {
                objects: row.iter().map(|s| s.object).collect(),
                rect: row.iter().skip(1).fold(row[0].rect, |acc, s| union_rect(acc, &s.rect)),
                frozen: false,
            })
            .collect()
    }


    /// The joined group a run belongs to, as its index into `joined_groups`.
    fn group_containing(&self, page: usize, object: usize) -> Option<usize> {
        self.tab().joined_groups
            .iter()
            .position(|group| group.page == page && group.objects.contains(&object))
    }

    /// **Forget every joined group of `pb`'s page that no longer names what it
    /// named**, and say how many were — the check that comes before any use of
    /// one. A group names runs by object number and a fingerprint of how they
    /// read when it was made (see [`JoinedGroup`]); it is kept only while the
    /// page still reads that way. A page change that renumbered the objects, or
    /// moved or retyped them, makes the group name other words, or none, and it
    /// is let go instead of opening an editor over the wrong text.
    fn forget_stale_groups(&mut self, pb: &PageBlocks) -> usize {
        let page = pb.page;
        let before = self.tab().joined_groups.len();
        self.tab_mut()
            .joined_groups
            .retain(|group| group.page != page || group_fingerprint(pb, &group.objects) == Some(group.fingerprint));
        let forgotten = before - self.tab().joined_groups.len();
        if forgotten > 0 {
            self.session_log.record(
                "pick-note",
                &format!("{forgotten} joined group(s) on page {} no longer match the page and were forgotten", page + 1),
            );
        }
        forgotten
    }

    /// Declare `objects` of `page` one paragraph by hand, remembering how they
    /// read now so that the declaration can be checked later (see
    /// [`JoinedGroup`]). `false` — nothing declared — when the page cannot be read
    /// or does not hold all of them.
    fn declare_group(&mut self, page: usize, objects: Vec<usize>) -> bool {
        let Ok((pb, _)) = self.page_blocks(page) else { return false };
        let Some(fingerprint) = group_fingerprint(&pb, &objects) else { return false };
        self.tab_mut().joined_groups.push(JoinedGroup { page, objects, fingerprint });
        true
    }

    /// Undeclare whatever join a run is part of — the whole of "Split the
    /// joined text". Nothing on the page changes: the grouping was the
    /// only thing there was to take back. `true` if a group was actually
    /// found and removed.
    fn split_group(&mut self, page: usize, object: usize) -> bool {
        match self.group_containing(page, object) {
            Some(index) => {
                self.tab_mut().joined_groups.remove(index);
                true
            }
            None => false,
        }
    }

    /// The text run under a point, read-only — no fallback tolerance and no
    /// side effects, unlike `pick_text_run`. Only what the right-click menu
    /// needs to ask "is a joined group sitting here".
    ///
    /// **`text_run_rects`, not `text_runs`.** The full read extracts every
    /// run's text, size and colour — real work across a whole page — for a
    /// question that only ever needed rectangles. Reported from use as the
    /// app freezing on right-click: this used the full read, called fresh
    /// on every repaint of an open popup (see `right_click_text_actions`'s
    /// own doc for the other half of that fix).
    fn text_run_object_at(&self, page: usize, at: AppPoint) -> Option<usize> {
        let doc = self.tab().doc.as_ref()?;
        let rects = doc.session.text_run_rects(page).ok()?;
        let (x, y) = (at.x as f32, at.y as f32);
        let area = |r: &pdf_core::document::Rect| ((r.right - r.left) * (r.bottom - r.top)).abs();
        rects
            .iter()
            .filter(|(_, r)| {
                x >= r.left.min(r.right)
                    && x <= r.left.max(r.right)
                    && y >= r.top.min(r.bottom)
                    && y <= r.top.max(r.bottom)
            })
            .min_by(|(_, a), (_, b)| area(a).total_cmp(&area(b)))
            .map(|(object, _)| *object)
    }

    /// **Declare the runs a text selection covers one paragraph**,
    /// overriding whatever the detector's own geometry would
    /// find — for blocks the automatic heuristic keeps apart on purpose (a
    /// different face, a gap wider than it allows, an unrelated column in
    /// between) that a person can see belong together anyway. Opens the
    /// same paragraph editor a click already opens automatically, so
    /// joining and then typing is one motion.
    ///
    /// The grouping itself is the whole edit: nothing on the page changes
    /// until the editor this opens is actually applied, so declaring the
    /// wrong runs joined costs nothing to reconsider — right-click one of
    /// them and split it back apart.
    ///
    /// Also reachable straight from a right-click on a multi-run
    /// selection — see the context menu's own "Join into one paragraph" —
    /// this is the ribbon's own "Link & Join Text" button's dispatch, the
    /// same "act at once on a selection already made, otherwise say how"
    /// shape `begin_web_link` already uses.
    fn begin_join_text(&mut self) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        if self.tab_mut().text_selection.is_some() {
            match self.join_selected_text() {
                Ok(message) => self.say_info(message),
                Err(e) => self.say_error(e),
            }
        } else {
            self.say_info(
                "join text — drag across the lines or blocks to join, then press this \
                 again (or right-click the selection and choose Join into one paragraph).",
            );
        }
    }

    fn join_selected_text(&mut self) -> Result<String, String> {
        let Some(range) = self.tab_mut().text_selection.clone() else {
            return Err("select text across at least two lines or blocks first.".into());
        };
        let page = self.tab_mut().selection_page;
        let rects = self.selection_rects(page, range);
        if rects.is_empty() {
            return Err("that selection has nothing to join.".into());
        }
        let selected_objects: Vec<usize> =
            self.runs_touched_by(page, &rects).iter().map(|r| r.object).collect();
        if selected_objects.len() < 2 {
            return Err("select text across at least two lines or blocks to join them.".into());
        }

        // Absorb any group already touching one of these runs, so joining a
        // third block onto an existing pair grows one group rather than
        // leaving two that overlap.
        let (pb, _) = self.page_blocks(page)?;
        self.forget_stale_groups(&pb);
        let mut objects = selected_objects.clone();
        self.tab_mut().joined_groups.retain(|group| {
            let touches = group.page == page && group.objects.iter().any(|m| selected_objects.contains(m));
            if touches {
                for member in &group.objects {
                    if !objects.contains(member) {
                        objects.push(*member);
                    }
                }
            }
            !touches
        });

        self.tab_mut().text_selection = None;
        if !self.declare_group(page, objects.clone()) {
            return Err("those words could not be read to be joined — nothing was changed.".into());
        }
        self.open_joined_editor(page, &objects, &pb)
    }

    /// Open the paragraph editor for a set of runs already declared joined
    /// — `pick_text_run`'s own automatic call, minus the geometry: the
    /// membership was decided once, at [`Self::join_selected_text`], and
    /// every later click just re-opens it.
    ///
    /// Read from the page's one cached read ([`Self::page_blocks`]) like a
    /// click is, not from a read of its own. Its lines are grouped by how the
    /// boxes overlap ([`Self::rows_from_selected`]): there is no detector line
    /// structure for objects somebody chose by hand.
    fn open_joined_editor(&mut self, page: usize, objects: &[usize], pb: &PageBlocks) -> Result<String, String> {
        let mut selected: Vec<Selected> = objects
            .iter()
            .filter_map(|object| {
                pb.runs.get(object).map(|r| Selected { page, object: *object, rect: r.rect, what: "the words" })
            })
            .collect();
        if selected.len() < 2 {
            return Err("that joined text is no longer on this page.".into());
        }
        selected.sort_by(|a, b| a.rect.top.min(a.rect.bottom).total_cmp(&b.rect.top.min(b.rect.bottom)));
        let runs: Vec<pdf_core::document::TextRun> =
            selected.iter().filter_map(|s| pb.runs.get(&s.object).cloned()).collect();
        let lines = Self::rows_from_selected(selected);
        self.pick_paragraph(page, lines, Vec::new(), &runs, &pb.faces, &pb.styles, &pb.shapes)
    }

    /// Match Properties: a selection already made becomes the sample if
    /// none is held yet, or is matched to the one already held — the same
    /// "act at once on a selection already made, otherwise arm and wait"
    /// shape `begin_web_link` uses, except the tool stays in hand afterward
    /// so a whole page's worth of mismatched runs can be fixed one
    /// selection after another.
    fn begin_match_properties(&mut self) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        if self.tab_mut().match_properties_sample.is_some() {
            match self.apply_match_properties_to_current_selection() {
                Ok(message) => self.say_info(message),
                Err(e) => self.say_error(e),
            }
            return;
        }
        if self.tab_mut().text_selection.is_some() {
            match self.match_properties_sample_from_current_selection() {
                Ok(message) => self.say_info(message),
                Err(e) => self.say_error(e),
            }
        } else {
            self.put_down_page_editors("armed match properties");
            self.tab_mut().markup_armed = None;
            self.tab_mut().link_armed = false;
            self.tab_mut().match_properties_armed = true;
            self.say_info(
                "match properties — drag across the sample text to copy from. Escape puts \
                 it down.",
            );
        }
    }

    /// Take the current selection as Match Properties' own sample, and arm
    /// it to match every selection made from here on — see
    /// [`Self::match_properties_sample`].
    fn match_properties_sample_from_current_selection(&mut self) -> Result<String, String> {
        let Some(range) = self.tab_mut().text_selection.clone() else {
            return Err("select the sample text first.".into());
        };
        let page = self.tab_mut().selection_page;
        if range.is_empty() {
            return Err("that selection has nothing to copy from.".into());
        }
        let Some(first) = self.run_at_selection_start(page, range.start) else {
            return Err("could not tell which run the selection starts in.".into());
        };
        self.tab_mut().match_properties_armed = false;
        self.tab_mut().text_selection = None;
        self.tab_mut().match_properties_sample = Some(self.build_match_properties_sample(page, &first));
        Ok(
            "match properties — sample set. Now drag across text to change; each selection \
             is matched right away. Escape puts the tool down."
                .into(),
        )
    }

    /// Everything [`Self::apply_match_properties_to_current_selection`] needs
    /// from the sample run, gathered once rather than before every target —
    /// see [`MatchPropertiesSample`]'s own doc for why.
    fn build_match_properties_sample(
        &self,
        page: usize,
        first: &pdf_core::document::TextRun,
    ) -> MatchPropertiesSample {
        // **One page-wide read, not one per run.** `run_font_name` opens the
        // page fresh to answer a single object's question; asking it once
        // per run on a real, busy page — a few hundred of them — was itself
        // a few hundred page-opens, reported from use as the app freezing
        // solid the moment "Match the font" (this is ported from) was
        // clicked. `run_font_names` answers all of them from one open.
        let names = self.tab().doc.as_ref().and_then(|d| d.session.run_font_names(page).ok()).unwrap_or_default();

        // **Every other run already on the page sharing the sample's own
        // font family**, ignoring the six-letter PDF subset tag on
        // `/BaseFont` (see `strip_subset_prefix`) — tried in turn when the
        // sample's own exact embedded copy cannot spell a target's text. A
        // subset is routinely cut to just the glyphs its own run used; the
        // same family embedded a *second* time elsewhere, for different
        // text, is often the only copy of that face on the page that
        // actually has the glyph needed. Reported from use: a heading in
        // "Montserrat-Light" and a lone digit run in "ArialMT" right next
        // to it — the heading's own subset had never needed a digit, but
        // the very same family, embedded again for a chart's axis labels
        // elsewhere on the page, had every one.
        //
        // One representative object per distinct on-page font *name* —
        // `registered_face_for_run` is idempotent (it skips re-registering
        // a name already known), but nothing is gained by asking it to
        // prove the same embedded copy still cannot spell a word nineteen
        // times because nineteen runs happen to share it.
        let family = names.get(&first.object).map(|n| strip_subset_prefix(n)).map(str::to_owned);
        let alternate_objects: Vec<usize> = match &family {
            Some(fam) => {
                let mut candidates: Vec<(usize, &String)> = names
                    .iter()
                    .filter(|(&object, name)| {
                        object != first.object && strip_subset_prefix(name) == fam.as_str()
                    })
                    .map(|(&object, name)| (object, name))
                    .collect();
                // Sorted so a rerun against an unchanged page tries the
                // same alternates in the same order — a `HashMap`'s own
                // iteration order is not that.
                candidates.sort_by_key(|(object, _)| *object);
                let mut seen_names: Vec<&String> = Vec::new();
                let mut objects = Vec::new();
                for (object, name) in candidates {
                    if seen_names.contains(&name) {
                        continue;
                    }
                    seen_names.push(name);
                    objects.push(object);
                }
                objects
            }
            None => Vec::new(),
        };
        let face = self.registered_face_for_run(page, first.object);
        MatchPropertiesSample {
            page,
            face,
            size: first.size,
            color: first.color,
            family,
            alternate_objects,
        }
    }

    /// **Rewrite every run the current selection touches to match the held
    /// sample's typeface, size and colour.**
    ///
    /// Two separate byte-safe edits per run, not one combined one: a face
    /// change and a colour change land on different fast paths in
    /// `set_text_run_styled` (see `set_run_color_in_stream`'s own doc), and
    /// asking for both in one `TextStyle` would fall through to PDFium's
    /// slower, riskier whole-page regeneration instead of either.
    fn apply_match_properties_to_current_selection(&mut self) -> Result<String, String> {
        let Some(sample) = self.tab_mut().match_properties_sample.clone() else {
            return Err("select the sample text first.".into());
        };
        let Some(range) = self.tab_mut().text_selection.clone() else {
            return Err("select the text to change.".into());
        };
        let page = self.tab_mut().selection_page;
        if page != sample.page {
            return Err("match properties: select text on the same page as the sample.".into());
        }
        if range.is_empty() {
            return Err("that selection has nothing to change.".into());
        }
        let rects = self.selection_rects(page, range);
        let targets = self.runs_touched_by(page, &rects);
        if targets.is_empty() {
            return Err("that selection has nothing to change.".into());
        }

        // **Re-resolved by its own origin before every edit, never
        // addressed by a remembered object number.** Confirmed against the
        // real CAMINO file: a single face change that has to embed a font
        // (`embed_typing_font`, the last-resort tier `set_run_in_stream`
        // falls to once neither the run's own font nor another already on
        // the page can spell the text) adds a new page object and shifts
        // every object number after it — one such swap turned a 306-object
        // page into 305. Addressing the next edit by the object number
        // `targets`/`runs_touched_by` reported *before* that swap risks
        // retyping whatever now happens to sit at that stale index —
        // reported from use as the words right after a matched run going
        // missing. And origin, not rect: a size change (the very next edit
        // here) or a font of different glyph widths moves a run's rect by
        // exactly the amount being asked for, so comparing it against a
        // value read before that change missed the run outright. The
        // origin — where the text actually *starts* — moves for neither
        // reason.
        // **One page-wide read, refreshed only when a face change actually
        // renumbers the page — not one `run_font_name` per target.** Asking
        // per target was itself the freeze this whole tool exists to fix
        // (see `build_match_properties_sample`'s own doc): a wide selection
        // can easily touch a few hundred runs. `names` stays valid for
        // every target that either already matches or fails every
        // alternate — neither renumbers anything — and is only stale for
        // the rest, which the loop already knows about because it just
        // asked for a fresh `current` for the same reason.
        let mut names = self.tab_mut().doc.as_ref().and_then(|d| d.session.run_font_names(page).ok()).unwrap_or_default();

        let mut faced = 0usize;
        for target in &targets {
            let Some(mut current) = self.run_at_origin(page, target.origin) else { continue };

            // **Already the same family — nothing to retype.** Skipped
            // rather than run through `embed_typing_font` anyway: that
            // path rewrites the run's own text-showing operator from
            // scratch, and doing that to a run that already matches is
            // both wasted work and, confirmed against the real CAMINO
            // file, a real risk on its own — two adjacent runs each
            // rewritten this way in the same pass corrupted the boundary
            // between them (a stray control character where the space
            // used to be), even though each rewrite alone was byte-safe.
            // A run already in the right family never needs that risk at
            // all.
            let current_family = names.get(&current.object).map(|n| strip_subset_prefix(n));
            let already_matches = current_family == sample.family.as_deref();
            if !already_matches {
                let mut this_one_faced = false;
                if let Some(name) = &sample.face {
                    let Some(doc) = &self.tab_mut().doc else { break };
                    let matched_face = doc.session.execute(pdf_core::command::Command::SetTextRun {
                        page_index: page,
                        object: current.object,
                        text: current.text.clone(),
                        style: pdf_core::document::TextStyle { face: Some(name.clone()), ..Default::default() },
                    });
                    this_one_faced = matched_face.is_ok();
                }
                if !this_one_faced {
                    for alt_object in &sample.alternate_objects {
                        let Some(alt_name) = self.registered_face_for_run(page, *alt_object) else {
                            continue;
                        };
                        let Some(doc) = &self.tab_mut().doc else { break };
                        let matched_face = doc.session.execute(pdf_core::command::Command::SetTextRun {
                            page_index: page,
                            object: current.object,
                            text: current.text.clone(),
                            style: pdf_core::document::TextStyle { face: Some(alt_name), ..Default::default() },
                        });
                        if matched_face.is_ok() {
                            this_one_faced = true;
                            break;
                        }
                    }
                }
                if this_one_faced {
                    faced += 1;
                    // The one re-resolve this loop needs: a face change
                    // just succeeded, which may have embedded a new font
                    // and shifted every object number after it — `names`
                    // included, so it is re-read here too rather than left
                    // to answer for whatever now happens to sit at its old
                    // keys.
                    let Some(fresh) = self.run_at_origin(page, target.origin) else { continue };
                    current = fresh;
                    names = self.tab_mut().doc.as_ref().and_then(|d| d.session.run_font_names(page).ok()).unwrap_or_default();
                }
            } else {
                faced += 1;
            }

            // **Only what is not already right.** A target selection can
            // legitimately touch the sample's own run again (an overlapping
            // drag, or the same span used for both in a one-off fix), and
            // rewriting a property that already holds the sample's own
            // value is exactly the unnecessary edit the face check above
            // exists to avoid — confirmed against the real CAMINO file: a
            // size edit applied to a run that already had that size still
            // rewrote its text-showing operator, and cost the space between
            // two adjacent numbers where nothing about the size had
            // actually changed.
            if current.size != sample.size {
                let Some(doc) = &self.tab_mut().doc else { break };
                let _ = doc.session.execute(pdf_core::command::Command::SetTextRun {
                    page_index: page,
                    object: current.object,
                    text: current.text.clone(),
                    style: pdf_core::document::TextStyle { size: Some(sample.size), ..Default::default() },
                });
            }
            if current.color != sample.color {
                let Some(doc) = &self.tab_mut().doc else { break };
                let _ = doc.session.execute(pdf_core::command::Command::SetTextRun {
                    page_index: page,
                    object: current.object,
                    text: current.text.clone(),
                    style: pdf_core::document::TextStyle { color: Some(sample.color), ..Default::default() },
                });
            }
        }
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;

        let count = targets.len();
        let plural = if count == 1 { "" } else { "s" };
        Ok(if sample.face.is_some() && faced == count {
            format!("matched the font on {count} run{plural}.")
        } else if sample.face.is_some() {
            format!(
                "matched size and colour on {count} run{plural}, and the typeface on {faced} \
                 of them — the rest use characters no embedded copy of that font on this page \
                 has a glyph for."
            )
        } else {
            format!(
                "matched size and colour on {count} run{plural} — the sample's own typeface \
                 has no embedded copy to give them, so only its size and colour carried over."
            )
        })
    }

    /// Ask for the document's own face to be used in the editor.
    ///
    /// Takes effect on the *next* frame — see `editor_face`. Returns nothing:
    /// a face that cannot be read is simply not installed, and the editor draws
    /// in the program's own font as it did before.
    ///
    /// Also reads which characters the face has ink for
    /// (`editor_face_coverage`), once per new face, so the editor can draw the
    /// rest in the program's own font instead of as blanks.
    fn want_document_face(&mut self, page: usize, object: usize) {
        self.want_document_face_for(page, object, None);
    }

    /// [`Self::want_document_face`] for a caller that knows which font resource
    /// the run is set in (`font`, the page's own identity for it — see
    /// `RunStyle::font`): **the face is then asked of the engine once per font
    /// per state of the document**, not once per click — see [`FaceCache`]. With
    /// `None` every call asks, as it always did.
    fn want_document_face_for(&mut self, page: usize, object: usize, font: Option<u32>) {
        let stamp = self.tab().doc.as_ref().map(|d| (d.id, d.render_epoch, d.session.undo_generation()));
        let key: Option<FaceKey> = font.zip(stamp).map(|(font, (id, epoch, generation))| (id, page, epoch, generation, font));
        let held = key.as_ref().and_then(|key| self.face_cache.get(key));
        let face = match held {
            Some(face) => face,
            None => {
                let asked = self.tab().doc.as_ref().map(|d| d.session.run_font_data(page, object));
                // "This run's font has no program" is an answer; a failed ask is
                // not a fact about the font, and is not kept.
                let answered = matches!(asked, Some(Ok(_)));
                let face = Self::read_face(asked.and_then(Result::ok).flatten());
                if let (Some(key), true) = (key, answered) {
                    self.face_cache.put(key, face.clone());
                }
                face
            }
        };

        let (Some(program), Some(metrics)) = (&face.program, face.metrics) else {
            self.no_document_face();
            return;
        };
        // Kept, not just checked: this is the font's real ascent/descent, in its
        // own 1000ths-of-an-em units — what `run_editor_font_size` sizes the
        // editor from when a run's own reported size cannot be trusted, instead of
        // a fixed guess that fits no particular face especially well.
        self.editor_face_metrics = Some(metrics);
        if self.editor_face == Some(face.key) {
            return;
        }
        // Replaced together with `editor_face`, and not used before
        // `editor_face_ready`, so the coverage always describes the face being
        // drawn.
        self.editor_face_coverage = face.coverage.clone();
        self.editor_face = Some(face.key);
        self.editor_face_ready = false;
        self.pending_face = Some(program.as_ref().clone());
    }

    /// The editor has no document face to draw in: the program's own face, as
    /// it was before any was asked for. Also what a click that does not ask for
    /// one leaves behind it — **the face of the last editor must not outlive it**.
    fn no_document_face(&mut self) {
        self.editor_face = None;
        self.editor_face_ready = false;
        self.editor_face_metrics = None;
        self.editor_face_coverage = None;
        self.pending_face = None;
    }

    /// Everything the editor needs of a font program, worked out once — see
    /// [`CachedFace`].
    ///
    /// **Checked before it is handed to the atlas builder.** A PDF may carry a
    /// Type 1 program, or a subset cut in a way nothing else reads; egui is not
    /// the place to find that out. **Which letters the face can really draw** is
    /// read here too, once per face: a subset keeps letters in its `cmap` it never
    /// drew, and the editor lays those out in the program's own face instead (see
    /// `editor_sections`) — parsing the face again for every character on every
    /// frame is not on; it is a set the layouter only looks up in.
    fn read_face(bytes: Option<Vec<u8>>) -> CachedFace {
        let Some(bytes) = bytes else {
            return CachedFace { program: None, metrics: None, key: 0, coverage: None };
        };
        let metrics = pdf_core::pdf::embed::metrics(&bytes);
        let coverage = metrics
            .and_then(|_| pdf_core::pdf::embed::outlined_chars(&bytes))
            .map(std::sync::Arc::new);
        CachedFace { key: font_key(&bytes), program: Some(std::rc::Rc::new(bytes)), metrics, coverage }
    }

    /// The scale [`Self::page_behind`] and [`Self::page_raster_for_sampling`]
    /// render at — a few tens of thousands of pixels for a whole page, small
    /// on purpose since the answer either function gives is a flat colour
    /// either way.
    const BACKGROUND_SAMPLE_SCALE: f32 = 0.25;

    /// The commonest colour inside `rect` of an already-rendered raster —
    /// the sampling half of [`Self::page_behind`], split out so
    /// [`Self::page_raster_for_sampling`]'s one render can answer this for
    /// many different rects. See `page_behind`'s own doc for why the
    /// commonest colour, not the average, and why sampled inside the box
    /// rather than beside it. `None` — no page to sample, same as a render
    /// that failed — is plain white, same as `page_behind` always answered.
    /// **`exclude` is the run's own text colour, not decoration.** The
    /// rect sampled is the text's own bounding box — there is no other
    /// rect to ask for "the background behind this paragraph" — so every
    /// glyph's own ink is inside the sample too, and could in principle
    /// out-vote the genuinely blank pixels between lines for a short
    /// enough, dense enough rect. Excluding pixels close to the text's own
    /// known colour before taking the mode keeps that from happening,
    /// correct regardless of whether the page is light-on-dark or
    /// dark-on-light.
    ///
    /// **Reported from use, with a screenshot, did not turn out to be this
    /// on the page it was reported from** — text hidden by recolouring it
    /// to its own paragraph's sampled "background" stayed faintly but
    /// clearly visible, and excluding the ink made no difference at all:
    /// the mode was `rgb(248,248,248)` with or without it, meaning the
    /// *whole* rect, blank gaps included, was already reading a few shades
    /// under white before any ink pixel was ever in contention. See the
    /// snap-to-white step below this match for the fix that actually
    /// addressed it. Kept anyway as a real, if evidently less common,
    /// failure mode of sampling the mode of a rect that is mostly text.
    fn background_at(
        raster: Option<&PageRaster>,
        rect: pdf_core::document::Rect,
        exclude: pdf_core::document::Color,
    ) -> pdf_core::document::Color {
        const WHITE: pdf_core::document::Color =
            pdf_core::document::Color { r: 255, g: 255, b: 255, a: 255 };
        // Half the distance from black to white on one channel: close
        // enough to catch the text's own ink and the darker half of its
        // anti-aliased edge, without reaching far enough to start
        // excluding a genuinely different background colour.
        const EXCLUDE_RADIUS: i32 = 64;
        let close_to_excluded = |r: u8, g: u8, b: u8| {
            let dr = r as i32 - exclude.r as i32;
            let dg = g as i32 - exclude.g as i32;
            let db = b as i32 - exclude.b as i32;
            dr * dr + dg * dg + db * db <= EXCLUDE_RADIUS * EXCLUDE_RADIUS
        };
        let Some(raster) = raster else { return WHITE };
        let scale = Self::BACKGROUND_SAMPLE_SCALE;
        let (width, height) = (raster.width as i32, raster.height as i32);

        let mut seen: std::collections::HashMap<(u8, u8, u8), usize> = Default::default();
        let (top, bottom) = (
            (rect.top.min(rect.bottom) * scale) as i32,
            (rect.top.max(rect.bottom) * scale) as i32,
        );
        let (from, to) = (
            (rect.left.min(rect.right) * scale) as i32,
            (rect.left.max(rect.right) * scale) as i32,
        );
        for y in top.max(0)..bottom.min(height - 1).max(top.max(0) + 1) {
            for x in from.max(0)..to.min(width - 1).max(from.max(0) + 1) {
                let at = ((y * width + x) * 4) as usize;
                if at + 2 < raster.pixels.len() {
                    let (r, g, b) = (raster.pixels[at], raster.pixels[at + 1], raster.pixels[at + 2]);
                    if close_to_excluded(r, g, b) {
                        continue;
                    }
                    // Quantised, so near-identical shades of one background
                    // count as the same background — see `quantize_to_nearest_8`.
                    let key = (quantize_to_nearest_8(r), quantize_to_nearest_8(g), quantize_to_nearest_8(b));
                    *seen.entry(key).or_default() += 1;
                }
            }
        }
        let mode = match seen.into_iter().max_by_key(|(_, count)| *count) {
            Some(((r, g, b), _)) => pdf_core::document::Color { r, g, b, a: 255 },
            // Every sampled pixel was close to the text's own colour — a
            // rect with no real background in it at all, not expected in
            // practice (a real line of text is never *all* ink). White is
            // the same fallback this had before the exclusion existed.
            None => return WHITE,
        };
        // **Checked against the exclusion above, on the real page this was
        // reported from: it made no difference at all.** The mode came
        // back `rgb(248,248,248)` whether or not pixels near the text's
        // own ink were counted — not a halo round individual glyphs, but
        // the *whole* sampled rect, gaps between lines included, reading a
        // few shades under pure white. That is `page_raster_for_sampling`'s
        // own lower-resolution render: a dense paragraph leaves barely any
        // run of pixels untouched by *some* nearby glyph's anti-aliasing at
        // that scale, so even the "blank" majority comes back slightly
        // darkened. A document's own background being *exactly* this close
        // to white without actually being white is not a real design
        // choice anyone makes on purpose — so a result this close to white
        // is snapped to it outright, rather than hidden text staying
        // faintly visible against a true white page forever because the
        // one rect available to sample it from is never pure white either.
        const NEAR_WHITE_RADIUS: i32 = 16;
        let from_white = (255 - mode.r as i32).pow(2) + (255 - mode.g as i32).pow(2) + (255 - mode.b as i32).pow(2);
        if from_white <= NEAR_WHITE_RADIUS * NEAR_WHITE_RADIUS {
            WHITE
        } else {
            mode
        }
    }

    /// What colour the page is around a run — one render, one rect. See
    /// [`Self::page_raster_for_sampling`] and [`Self::background_at`] for the
    /// two halves of this, split apart for a caller that samples many rects
    /// on the same page and does not want to pay for a render each time.
    fn page_behind(
        &self,
        page: usize,
        rect: pdf_core::document::Rect,
        text_color: pdf_core::document::Color,
    ) -> pdf_core::document::Color {
        Self::background_at(self.page_raster_for_sampling(page).as_deref(), rect, text_color)
    }

    /// Whether a click landed on type the page *draws* as shapes: a path the
    /// size of a letter, within the slack a click on a word gets.
    ///
    /// Asked only after [`Self::drawn_word_at`] found nothing it could read, so
    /// what is left to tell apart is "drawn letters, in no typeface on hand"
    /// from "nothing here at all". Judged by the shape test classification and
    /// redaction already use (`looks_like_type`), so the three cannot disagree
    /// about what a letter is.
    ///
    /// **A known ceiling:** a producer that draws a whole line as one path has
    /// no letter-sized path to find, and a `DrawnObject` carries no segment
    /// count to tell a line of type from a rounded box, so the count is passed
    /// as 0 and only per-letter paths count. Such a page is told "click on some
    /// words" — true, if less helpful than the typeface advice. Carrying the
    /// segment count on `DrawnObject` closes it.
    fn click_is_on_drawn_type(&mut self, page: usize, at: AppPoint) -> bool {
        let Some(size) = self.tab().doc.as_ref().and_then(|d| d.session.page_size(page).ok()) else {
            return false;
        };
        let near = HIT_TOLERANCE_PT as f32;
        let (x, y) = (at.x as f32, at.y as f32);
        self.layers_on(page).iter().any(|d| {
            let (left, right) = (d.rect.left.min(d.rect.right), d.rect.left.max(d.rect.right));
            let (top, bottom) = (d.rect.top.min(d.rect.bottom), d.rect.top.max(d.rect.bottom));
            d.kind == pdf_core::document::DrawnKind::Shape
                && x >= left - near
                && x <= right + near
                && y >= top - near
                && y <= bottom + near
                && pdf_core::document::classify::looks_like_type(
                    right - left,
                    bottom - top,
                    0,
                    size.width_pt,
                    size.height_pt,
                )
        })
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

        self.tab_mut().editing_run = Some(EditingRun {
            page,
            // No page object owns these words; they are shapes.
            object: usize::MAX,
            look_object: usize::MAX,
            original: found.text.clone(),
            rect: found.rect,
            lines: vec![(vec![usize::MAX], found.rect)],
            frozen: vec![false],
            twins: Vec::new(),
            doc_generation: self.doc_generation(),
            render_epoch: self.render_epoch(),
            refusal: None,
            buffer: found.text.clone(),
            style: pdf_core::document::TextStyle {
                size: Some(placed.size),
                color: Some(pdf_core::document::Color { r: 20, g: 20, b: 20, a: 255 }),
                at: Some((placed.left, placed.baseline)),
                face: None,
            },
            was: pdf_core::document::TextStyle::default(),
            current_face: None,
            background: self.page_behind(page, found.rect, pdf_core::document::Color { r: 20, g: 20, b: 20, a: 255 }),
            drawn: true,
            focused: false,
            box_resize: RunBox::default(),
        });
        Some(
            "these words are drawn, not written — type converted to outlines. \
             Changing them takes the drawn shapes off the page and writes real text \
             in their place, in a face that will not match. Escape to leave them."
                .into(),
        )
    }

    /// The face a new line grown out of `edit` should be written in.
    ///
    /// Tried in order: a face already picked or read off the run, but only
    /// if it is one the app already has registered for typing — the case
    /// `outlinedfont add` or an earlier font pick covers. **Reported from
    /// use, with a screenshot: a line added under a bold heading came back
    /// in a plain, unrelated weight** — for an ordinary embedded document
    /// font, which almost never happens to be one of the app's own
    /// registered faces, that first lookup fails silently and always did,
    /// so every added line fell all the way back to plain Helvetica
    /// regardless of what the heading actually looked like. Retyping the
    /// run's own first line never had this problem, because that path
    /// rewrites the existing text object in place and so simply reuses
    /// whatever font was already there — this does the same thing by hand
    /// for the *new* line, pulling the run's own font program straight out
    /// of the file (see [`pdf_core::document::Document::run_font_data`])
    /// and registering it under a name of its own, rather than asking the
    /// app's unrelated typing-font list to happen to already have it.
    fn registered_face_of(&self, edit: &EditingRun) -> Option<String> {
        if let Some(name) = [edit.style.face.as_deref(), edit.current_face.as_deref()]
            .into_iter()
            .flatten()
            .find(|name| pdf_core::text::is_registered(name))
        {
            return Some(name.to_string());
        }

        // The font of the object the box takes its look from — the same one
        // its size and colour came from, and the one `current_face` names —
        // not whichever object happens to be first in the paragraph (a scrap
        // in another weight can be).
        let source = if edit.look_object == usize::MAX { edit.object } else { edit.look_object };
        self.registered_face_of_object(edit.page, source)
    }

    /// The font program object `object` of `page` is drawn in, registered for
    /// writing with and named.
    fn registered_face_of_object(&self, page: usize, object: usize) -> Option<String> {
        let bytes = self.tab().doc.as_ref()?.session.run_font_data(page, object).ok()??;
        // Named from its own bytes, not the run: many runs on a page share
        // one font, and this way they share one registration too instead of
        // piling up a copy per run edited in a session.
        let key = font_key(&bytes);
        let name = format!("run-own-font-{key:x}");
        if !pdf_core::text::is_registered(&name) {
            pdf_core::text::register(&name, bytes).ok()?;
        }
        Some(name)
    }

    /// Write lines added below where an edit's own text used to end, in
    /// that edit's own size, colour and font rather than a fixed default —
    /// so a line grown out of a heading or a paragraph looks like it
    /// belongs with the rest of it, not like a different piece of text that
    /// happens to sit underneath.
    ///
    /// Reported from use: growing a single run into two lines wrote the
    /// second in `write_text_at`'s own flat default (Helvetica, 14pt, dark
    /// grey) regardless of what the first line actually looked like — the
    /// same gap this closes for a paragraph that grows past its own last
    /// line.
    fn write_extra_styled_lines(
        &mut self,
        page: usize,
        base_x: f32,
        mut y: f32,
        gap: f32,
        style: &pdf_core::document::TextStyle,
        face: Option<&str>,
        extra_lines: &[&str],
    ) -> Result<(), String> {
        let size = style.size.unwrap_or(12.0);
        let color = style
            .color
            .unwrap_or(pdf_core::document::Color { r: 20, g: 20, b: 20, a: 255 });
        for extra in extra_lines {
            y += gap;
            if extra.is_empty() {
                continue;
            }
            self.write_styled_line_at(page, (base_x, y), extra, size, color, face)?;
        }
        Ok(())
    }


    /// A paragraph's own runs, keyed by object id, so a colour-only
    /// `SetTextRuns` colour edit can send back the exact text
    /// each object already has rather than guessing at it.
    ///
    /// **`text_runs_some`, not `text_runs`.** The full read extracts every
    /// run's own words — real PDFium work, once per run, for every run on
    /// the page — to answer a question this paragraph's own dozen or so
    /// objects already settle. Reported from use: a paragraph apply still
    /// took most of a second after `set_runs_in_stream`'s own batching had
    /// already brought the actual write down to under 100ms — flat,
    /// regardless of the paragraph's own size, which matches this exact
    /// cost and not the write. `wanted` is `edit.lines`' own objects,
    /// flattened — never the whole page.
    fn current_text_by_object(
        &self,
        page: usize,
        wanted: &std::collections::HashSet<usize>,
    ) -> std::collections::HashMap<usize, String> {
        self.tab()
            .doc
            .as_ref()
            .and_then(|doc| doc.session.text_runs_some(page, wanted).ok())
            .map(|runs| runs.into_iter().map(|r| (r.object, r.text)).collect())
            .unwrap_or_default()
    }


    /// Commit the one paragraph open for editing, if it actually asks for
    /// anything. **`true` when the engine refused the edit and the editor was
    /// kept open** — with the typing in it and the reason on show next to Apply
    /// — rather than closed with the person's words thrown away.
    ///
    /// **A refusal changes nothing and costs nothing.** What the engine refuses
    /// it refuses whole, before anything is written, and the page is as it was
    /// (the paragraph apply is atomic, a colour written ahead of the replace is
    /// taken back with it); so the box stays, and its words with it, for the
    /// person to correct or to let go of (see [`Self::leave_editor_by_click`] and
    /// [`EditingRun::refusal`]). What was refused is told from what the apply
    /// left behind: it said an error, and the document's history did not move. An
    /// apply that got part of the way — a drawn word taken off whose replacement
    /// would not go on, new lines that could not be added — moved the history, and
    /// its editor is **not** brought back: those words are on the page now.
    fn apply_editing_page(&mut self) -> bool {
        let Some(mut edit) = self.tab_mut().editing_run.take() else { return false };
        // A box dragged by its left edge has moved the run with it: the new
        // start is part of what is applied. From where the run started, not
        // from `style.at`, so applying it again after a refusal is the same.
        if edit.box_resize.left_shift_pt != 0.0 {
            if let Some((x, y)) = edit.was.at {
                let y = edit.style.at.map_or(y, |(_, y)| y);
                edit.style.at = Some((x + edit.box_resize.left_shift_pt, y));
            }
        }
        if !self.edit_has_changes(&edit) {
            self.say_info("left as it was.");
            return false;
        }
        // **Not written if the page changed since this was picked.** The editor
        // holds page-object numbers, and an edit, an undo, a redo — or any of the
        // changes that are not in the history, a restack, an import, a lock — in
        // between can renumber them: the typed words would land on other objects.
        // The editor is closed, and this says why nothing was written rather than
        // guessing which objects are the ones picked.
        if self.editor_is_stale(&edit) {
            self.let_go_of_stale_editor(&edit);
            return false;
        }
        // **Timed and logged on its own, separate from whatever the session
        // log's own "editing a paragraph…" → "paragraph changed" gap shows.**
        // That gap also counts however long the words themselves took to
        // type, which can run to several seconds on its own — on a report
        // of apply itself still feeling slow, the session log alone cannot
        // tell the two apart. This logs only the computation.
        let started = std::time::Instant::now();
        let (generation, errors_before) = (self.doc_generation(), self.errors_said);
        let backup = edit.clone();
        self.apply_one_edit(edit);
        let elapsed = started.elapsed();
        self.session_log.record("info", &format!("apply took {elapsed:?}"));

        // Refused: an error was said, and nothing was executed. (An error said
        // *after* something was written is not a refusal — see above.)
        if self.errors_said == errors_before || self.doc_generation() != generation {
            return false;
        }
        let refused = self.cmd.history().iter().rev().find(|said| said.kind == Kind::Error).map(|said| said.text.clone());
        let Some(reason) = refused else { return false };
        // Put back as it was, **stamped for the page as it is now**: the apply
        // marks the page stale whether or not it got anywhere (a safety net — the
        // renders and readings made before it must not be trusted), which moved
        // the epoch; the page itself is unchanged, and the box has to stay good.
        let mut kept = backup;
        kept.render_epoch = self.render_epoch();
        kept.refusal = Some(EditRefusal { reason, buffer: kept.buffer.clone(), style: kept.style.clone() });
        self.tab_mut().editing_run = Some(kept);
        true
    }


    /// The open document's command-history generation, or `0` with none open —
    /// what an [`EditingRun`] is stamped with when it is picked.
    fn doc_generation(&self) -> u64 {
        self.tab().doc.as_ref().map(|d| d.session.undo_generation()).unwrap_or(0)
    }

    /// The open document's [`Doc::render_epoch`], or `0` with none open — the other
    /// half of what an [`EditingRun`] is stamped with.
    fn render_epoch(&self) -> u64 {
        self.tab().doc.as_ref().map(|d| d.render_epoch).unwrap_or(0)
    }

    /// The state of the open document that anything read from `page` — its
    /// paragraphs, its drawn words, a joined group's members — is good for:
    /// `(page, render epoch, undo generation)`. The history generation moves for
    /// every command, the epoch for every change of the page the history does not
    /// see (and a few that are no change at all); a reading made under one stamp is
    /// not used under another.
    fn doc_stamp(&self, page: usize) -> (usize, u64, u64) {
        (page, self.render_epoch(), self.doc_generation())
    }

    /// Whether the page an open editor was picked from is no longer the page it
    /// was picked from — see [`EditingRun::render_epoch`] and
    /// [`EditingRun::doc_generation`] — or is not there at all.
    fn editor_is_stale(&self, edit: &EditingRun) -> bool {
        let Some(doc) = self.tab().doc.as_ref() else { return true };
        edit.doc_generation != doc.session.undo_generation()
            || edit.render_epoch != doc.render_epoch
            || edit.page >= doc.page_count
    }

    /// An editor whose page changed under it is closed, and says why; what was
    /// typed in it goes to the clipboard, so that nothing a person typed is just
    /// lost to a change they did not see coming.
    fn let_go_of_stale_editor(&mut self, edit: &EditingRun) {
        let typed = !edit.buffer.trim().is_empty();
        self.offer_typed_text(edit);
        self.say_error(format!(
            "{STALE_EDITOR_MESSAGE}{}",
            if typed { " What was typed is on the clipboard." } else { "" }
        ));
    }

    /// Words a person typed that are about to be let go of unapplied, put on the
    /// clipboard at the next frame (see [`Self::text_to_offer`]).
    fn offer_typed_text(&mut self, edit: &EditingRun) {
        if !edit.buffer.trim().is_empty() {
            self.text_to_offer = Some(edit.buffer.clone());
        }
    }

    /// Per frame: an open editor whose page changed under it is closed.
    fn close_editor_if_stale(&mut self) {
        if !self.tab().editing_run.as_ref().is_some_and(|edit| self.editor_is_stale(edit)) {
            return;
        }
        if let Some(edit) = self.tab_mut().editing_run.take() {
            if self.edit_has_changes(&edit) {
                self.let_go_of_stale_editor(&edit);
            } else {
                self.say_info("left as it was.");
            }
        }
    }

    /// Whether an edit actually asks for anything — same words, same style,
    /// nothing to do.
    fn edit_has_changes(&self, edit: &EditingRun) -> bool {
        let typed = edit.buffer.trim();
        if typed.is_empty() {
            // Typed away is a deletion — a change, unless there was nothing to delete.
            return !edit.original.trim().is_empty();
        }
        let changed_look = pdf_core::document::TextStyle { face: None, ..edit.style.clone() }
            != pdf_core::document::TextStyle { face: None, ..edit.was.clone() };
        let picked_font = edit.style.face.is_some();
        !(typed == edit.original.trim() && !changed_look && !picked_font)
    }

    /// One `SetTextRun` per object per line, exactly the shape
    /// `apply_paragraph_edit` writes — collected instead of executed one at
    /// a time, so the whole paragraph can be sent as one `Command::SetTextRuns`
    /// and land (and undo) as a single step, however many lines it has.
    ///
    /// **Lines are matched by content, not by position.** This used to zip
    /// `typed.split('\n')` against `edit.lines` purely by index: line `i`'s
    /// typed text always went to line `i`'s own object. That breaks the
    /// instant a `\n` is inserted or removed anywhere except the very last
    /// line — Enter pressed mid-paragraph, or a Backspace that merges two
    /// lines, shifts every later line's typed text onto the *previous*
    /// line's object one position early, and the true last line falls off
    /// the end into the overflow path in the wrong font. **Reported from
    /// use, twice**: a paragraph edited near its end came back with a wrong
    /// colour ghosted over one word and a stray hyphen (fixed in 0.1.11 for
    /// the one trigger then known, the auto-wrap feature's own insertion),
    /// then the identical shape came back from a plain Enter press, which
    /// that fix never touched — Enter inserting a line mid-paragraph is its
    /// own supported feature, not something this could just refuse.
    ///
    /// The fix is a line-level diff against `edit.original` (guaranteed, by
    /// `join_paragraph_lines`, to split back into exactly `edit.lines.len()`
    /// lines): the longest common prefix and longest common suffix of
    /// unchanged lines are found first, so only the lines strictly between
    /// them — the ones the edit actually touched — are ever reassigned.
    /// Every line after the touched region keeps mapping to its own
    /// original object, whichever index that now falls at on either side.
    /// Only a genuine net growth inside that touched middle (not merely at
    /// the paragraph's own end) has nowhere existing to go, and is written
    /// as new objects the same way text typed past the last line already
    /// was — now anchored whereever the touched region itself ends, not
    /// always the paragraph's own bottom.
    ///
    /// **Not every line is written: the lines a person did not change are left
    /// alone, and a frozen line is never touched.** [`plan_paragraph_edit`]
    /// decides, line by line, from the text alone, and [`line_edits`] turns the
    /// plan into what the engine is told (see the `paragraph_lines` module's own
    /// doc for why a retyped line *replaces* its pieces):
    /// * `Kept` — says what it said: not mentioned at all, so a justified line
    ///   of three to eight pieces stays three to eight pieces;
    /// * `Written` — a `TextLineEdit::Retype`: the new words onto the line's
    ///   first object and **the line's other pieces removed from the page**;
    /// * `Removed` — typed away: a `TextLineEdit::Remove` of every piece;
    /// * `Frozen` — carries words the page draws as shapes (see
    ///   [`EditingRun::frozen`]): left exactly as it is whatever was typed over
    ///   it, named in the session log, and counted so the apply can say how many
    ///   had different words typed over them.
    ///
    /// The result is one `Command::ReplaceTextLines`, which undoes by a page
    /// snapshot (exact) and **renumbers the page's objects** — afterwards every
    /// object id kept from before is stale. So a new colour for the paragraph,
    /// which names objects by id, goes first in the same list (see
    /// [`commands_for`]), and the lines typed past the paragraph's own end are
    /// not written here at all but returned as [`SurplusLines`], to be written
    /// by position once the replace has run.
    ///
    /// **A paragraph's colour is applied, its position is not.** The panel
    /// offers both for a paragraph, and neither was ever applied to one (only
    /// the font and the size are said once for a whole block). Colour is: every
    /// piece that stays on the page takes it (see [`recolour_targets`]).
    /// Position is not — a paragraph is several lines and has no single place to
    /// move to — and the apply says so rather than dropping it silently.
    ///
    /// **Refuses, in release too, when the pieces do not line up** — see
    /// [`check_lines_up`]. The refusal comes before anything is built, so the
    /// page is as it was.
    fn paragraph_edit_commands(&mut self, edit: &EditingRun, typed: &str) -> Result<ParagraphCommands, String> {
        self.paragraph_edit_commands_with(edit, typed, STRETCH_JUSTIFIED_LINES)
    }

    /// [`Self::paragraph_edit_commands`], with whether a retyped line of a
    /// justified paragraph is asked to keep the width it had **said by the caller**:
    /// the apply asks first, and when the engine cannot stretch some line (it
    /// refuses the whole batch and does not say which) asks again without.
    fn paragraph_edit_commands_with(
        &mut self,
        edit: &EditingRun,
        typed: &str,
        stretch: bool,
    ) -> Result<ParagraphCommands, String> {
        let original_lines: Vec<&str> = edit.original.split('\n').collect();
        check_lines_up(original_lines.len(), &edit.lines, &edit.twins, &edit.frozen)?;
        let n = edit.lines.len();
        let new_lines: Vec<&str> = typed.split('\n').collect();

        // The font and the size travel on every retyped line; a colour and a
        // position never do — see `paragraph_lines::line_edits`.
        let style = pdf_core::document::TextStyle {
            face: edit.style.face.clone(),
            size: (edit.style.size != edit.was.size).then_some(edit.style.size).flatten(),
            ..Default::default()
        };
        // A new size or font has to reach every line, so none may be passed
        // over as unchanged.
        let restyle = style.face.is_some() || style.size.is_some();
        let plan = plan_paragraph_edit(&original_lines, &edit.frozen, &new_lines, restyle);

        // **The paragraph's own words, for the colour edits only.** A colour is
        // written as the object's current words with a new colour (the engine's
        // byte-safe path) and so needs them read back — and only for the objects
        // it will name, see `current_text_by_object`'s own doc. Nothing is read
        // at all unless a colour was asked for: the replace itself names objects,
        // never words.
        let mut recolour = Vec::new();
        if let Some(color) = (edit.style.color != edit.was.color).then_some(edit.style.color).flatten() {
            let targets = recolour_targets(&plan.fates, &edit.lines);
            let wanted: std::collections::HashSet<usize> = targets.iter().copied().collect();
            let current_text = if wanted.is_empty() {
                std::collections::HashMap::new()
            } else {
                self.current_text_by_object(edit.page, &wanted)
            };
            for object in targets {
                let Some(text) = current_text.get(&object) else {
                    return Err(format!(
                        "this paragraph no longer lines up with what was picked (object {object} \
                         could not be read), so nothing was changed — pick it again."
                    ));
                };
                recolour.push((
                    object,
                    text.clone(),
                    pdf_core::document::TextStyle { color: Some(color), ..Default::default() },
                ));
            }
        }

        self.session_log.record("info", &plan_log_line(new_lines.len(), &plan, &edit.lines, &edit.twins));
        // A retyped line of a justified paragraph is asked to keep the width it
        // had — the editor's own test for drawing it justified, and that its
        // lines end at one margin (left-aligned text starts at one margin too).
        let justified = stretch && paragraph_should_justify(&edit.lines) && ends_at_one_margin(&edit.lines);
        let commands =
            commands_for(edit.page, recolour, line_edits(&plan.fates, &edit.lines, &edit.twins, &style, justified));

        // Surplus typed lines in the touched middle with no original line
        // left to hold them — same shape as text typed past the paragraph's
        // own last line, just anchored wherever the touched region ends
        // rather than always the very bottom.
        let surplus = (!plan.surplus.is_empty()).then(|| {
            let anchor_rect = edit.lines[plan.surplus_below].1;
            let gap = if n >= 2 {
                (edit.lines[1].1.top - edit.lines[0].1.top).abs().max(1.0)
            } else {
                (anchor_rect.bottom - anchor_rect.top).max(12.0)
            };
            SurplusLines {
                base_x: edit.style.at.map(|(x, _)| x).unwrap_or(anchor_rect.left),
                below: anchor_rect.bottom,
                gap,
                face: self.registered_face_of(edit),
                lines: plan.surplus.iter().map(|line| line.to_string()).collect(),
            }
        });
        Ok(ParagraphCommands {
            commands,
            frozen_changed: plan.frozen_changed,
            position_ignored: edit.style.at != edit.was.at,
            surplus,
        })
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


    /// Write words onto the page, where the next click lands.
    ///
    /// **Real text objects, not a picture of text.** They select, search and
    /// copy like anything else on the page — which is the whole reason the
    /// engine writes text rather than drawing letters, and the reason this is
    /// not simply an ink stroke shaped like writing.
    fn add_text(&mut self, text: String) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let text = text.trim().to_string();
        let page = self.tab_mut().page;
        // **Reported from use: adding text meant typing the words into the
        // command box before knowing where they would land** — "totally
        // confusing and unintuitive". Bare `addtext` now drags out a box to
        // type into instead (see `begin_text_box`); `addtext <words>` is
        // kept exactly as it was, for a script or anyone who prefers typing
        // the words first and placing them with one click.
        if text.is_empty() {
            self.arm(Tool::PlaceText, page);
            return;
        }
        self.arm(Tool::Write(text), page);
    }

    /// Put words on the page at `at`.
    fn write_text_at(&mut self, page: usize, at: AppPoint, text: &str) -> Result<String, String> {
        use pdf_core::document::{Annotation, Color, Glyph};

        const SIZE: f32 = 14.0;
        let id = self.tab_mut().next_text_id;
        self.tab_mut().next_text_id += 1;
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };

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

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }

        Ok(format!("wrote \"{text}\" on page {}.", page + 1))
    }

    /// Below this, `a` and `b` are a click that barely moved before letting
    /// go, not a box someone meant to draw — the same kind of noise floor a
    /// negligible drag on a shape or a picture is measured against. A box
    /// bigger than this but still small gets typed into anyway: the font
    /// size shrinks to fit it instead of refusing (see below).
    const MIN_TEXT_BOX_PT: f32 = 2.0;

    /// Start composing brand new text in the box `a`..`b` describes.
    ///
    /// Nothing is written to the page yet — that happens once in
    /// [`Self::apply_new_text_box`], when there is something to write. Until
    /// then this only opens [`Self::new_text_box`], the same way arming any
    /// other tool clears whatever else was selected first: a text box and a
    /// run/shape selection are never live together.
    fn begin_text_box(&mut self, page: usize, a: AppPoint, b: AppPoint) -> Result<String, String> {
        if self.tab_mut().doc.is_none() {
            return Err("nothing open.".into());
        }
        let rect = pdf_core::document::Rect {
            left: a.x.min(b.x) as f32,
            right: a.x.max(b.x) as f32,
            top: a.y.min(b.y) as f32,
            bottom: a.y.max(b.y) as f32,
        };
        if (rect.right - rect.left) < Self::MIN_TEXT_BOX_PT || (rect.bottom - rect.top) < Self::MIN_TEXT_BOX_PT {
            return Err("text: that box is too small to type into.".into());
        }

        // **Was a bare `self.tab_mut().editing_run = None`, the only place in
        // the file that discarded an open run editor without going through
        // `put_down_page_editors`.** Every other tool switch that can
        // abandon an in-progress edit — Escape, a different tool, re-arming
        // Edit Text — logs "left as it was." so the session log always shows
        // what happened to it. This one didn't: drawing a brand new text box
        // while a paragraph was still open silently dropped it with no
        // trace, which is exactly the shape of gap that made a reported
        // corruption impossible to confirm or rule out from the log alone.
        self.put_down_page_editors("started a new text box");
        self.tab_mut().selected = None;
        self.tab_mut().group.clear();
        if let Some(layer) = self.tab_mut().markup.existing_mut(page) {
            layer.clear_selection();
        }
        self.tab_mut().signature_selected = None;
        self.tab_mut().placed_image_selected = None;

        // The usual default, unless the box itself is smaller than that in
        // either direction — then the size follows the box down instead of
        // the box being refused for not fitting a size nobody asked for.
        let size = (rect.right - rect.left).min(rect.bottom - rect.top).min(14.0);

        self.tab_mut().new_text_box = Some(NewTextBox {
            page,
            rect,
            buffer: String::new(),
            size,
            color: pdf_core::document::Color { r: 20, g: 20, b: 20, a: 255 },
            face: None,
            align: TextAlign::Left,
            focused: false,
        });
        Ok("type into the box — the panel on the right sets its look, and Add to Page adds it to the page.".into())
    }

    /// One line of styled text, written at `origin` (the baseline, left
    /// edge) — the primitive [`Self::apply_new_text_box`] calls once per
    /// row its box wrapped to.
    ///
    /// Mirrors `replace_outlined_word`'s font-vs-no-font split: a picked
    /// face is shaped for its own glyph advances, the same as anything else
    /// this app writes in a font that is not one of the PDF standard 14. No
    /// pick keeps `write_text_at`'s simple single-run `Annotation::Text` —
    /// Helvetica's metrics are the reader's own PDF viewer's to know, so
    /// nothing here has to compute them.
    fn write_styled_line_at(
        &mut self,
        page: usize,
        origin: (f32, f32),
        text: &str,
        size: f32,
        color: pdf_core::document::Color,
        face: Option<&str>,
    ) -> Result<(), String> {
        let command = self.styled_line_command(page, origin, text, size, color, face)?;
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session.execute(command).map_err(|e| format!("{e}"))?;
        Ok(())
    }

    /// The command [`Self::write_styled_line_at`] executes, **not yet run**, so
    /// that several lines can go in as one `Command::Batch` and undo as one.
    fn styled_line_command(
        &mut self,
        page: usize,
        origin: (f32, f32),
        text: &str,
        size: f32,
        color: pdf_core::document::Color,
        face: Option<&str>,
    ) -> Result<pdf_core::command::Command, String> {
        use pdf_core::document::{Annotation, Glyph};

        let (font, font_asset, glyphs) = match face {
            Some(name) => {
                let shaped = pdf_core::text::shape(name, text)
                    .map_err(|e| format!("{name} could not set those words — {e}"))?;
                let mut boundaries: Vec<usize> =
                    shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
                boundaries.sort_unstable();
                boundaries.dedup();

                let mut pen = origin.0;
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
                        ch: text.get(from..to).unwrap_or_default().to_string(),
                        id: glyph.id,
                        x: pen + glyph.offset_x * size,
                        y: origin.1 - glyph.offset_y * size,
                        radians: 0.0,
                    });
                    pen += glyph.advance * size;
                }
                (name.to_string(), Some(name.to_string()), placed)
            }
            None => (
                "Helvetica".to_string(),
                None,
                vec![Glyph { ch: text.to_string(), id: 0, x: origin.0, y: origin.1, radians: 0.0 }],
            ),
        };

        let id = self.tab_mut().next_text_id;
        self.tab_mut().next_text_id += 1;
        Ok(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: Annotation::Text {
                text: text.to_string(),
                font,
                font_asset,
                size,
                color,
                glyphs,
                id,
                restore: String::new(),
                frame: Vec::new(),
                frame_width: 0.0,
            },
        })
    }

    /// Turn [`Self::new_text_box`]'s typed buffer into real page content,
    /// one line per row it wrapped to on screen.
    ///
    /// The wrap is *reproduced* here rather than read back from the widget
    /// that drew it — laying the same text out again, at the same width and
    /// size, gives the exact rows `egui::TextEdit` showed while it was
    /// being typed (see `LayoutJob`'s own wrapping, which is what the
    /// widget uses internally too), so what lands on the page is what was
    /// seen in the box.
    fn apply_new_text_box(&mut self, ui: &egui::Ui) {
        let Some(new_text) = self.tab_mut().new_text_box.take() else { return };
        let typed = new_text.buffer.trim();
        if typed.is_empty() {
            self.say_info("nothing typed — the box was left empty.");
            return;
        }
        let page = new_text.page;
        // Page points per screen pixel at the moment Add to Page was
        // pressed — the same conversion the box itself was drawn with, one
        // frame earlier. A zoom between typing and pressing it would shift
        // the wrap very slightly; not worth guarding against for how rare
        // and how small a miss that is.
        let scale = self.tab_mut().last_view.map(|v| v.scale).unwrap_or(1.0).max(0.01);
        let box_width_pt = new_text.rect.right - new_text.rect.left;
        let size_px = (new_text.size * scale).max(1.0);

        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = (box_width_pt * scale).max(1.0);
        job.append(
            typed,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(size_px),
                color: egui::Color32::BLACK,
                ..Default::default()
            },
        );
        let galley = ui.fonts_mut(|f| f.layout_job(job));

        let face = new_text.face.as_deref();
        let mut lines_written = 0u32;
        let mut failed: Option<String> = None;
        for placed in &galley.rows {
            let text = placed.row.text();
            let trimmed = text.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let row_width_pt = placed.row.size.x / scale;
            let x = match new_text.align {
                TextAlign::Left => new_text.rect.left,
                TextAlign::Center => new_text.rect.left + (box_width_pt - row_width_pt) / 2.0,
                TextAlign::Right => new_text.rect.right - row_width_pt,
            };
            // The glyph closest to the row's own top gives its ascent — the
            // same value for every glyph in the row, since the whole box is
            // one font and size. A blank row (skipped above, `continue`)
            // would have none to ask.
            let ascent_px =
                placed.row.glyphs.first().map(|g| g.font_ascent).unwrap_or(size_px * 0.8);
            let baseline_y = new_text.rect.top + (placed.pos.y + ascent_px) / scale;

            match self.write_styled_line_at(
                page,
                (x, baseline_y),
                trimmed,
                new_text.size,
                new_text.color,
                face,
            ) {
                Ok(()) => lines_written += 1,
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }

        match failed {
            Some(e) => self.say_error(e),
            None if lines_written == 1 => {
                self.say_info(format!("text added to page {}.", page + 1))
            }
            // Each line is its own `AddAnnotation` — several steps, not
            // one, the same as `replace_outlined_word`'s own "this took two
            // steps" — so undoing it back off takes the same number back.
            None if lines_written > 1 => self.say_info(format!(
                "text added to page {} as {lines_written} lines — `undo` {lines_written} times puts it all back.",
                page + 1
            )),
            None => self.say_info("nothing typed — the box was left empty."),
        }
    }

    fn add_image_dialog(&mut self) {
        let dialog = rfd::FileDialog::new()
            .set_title("Add an image")
            .add_filter("Picture", &["png", "jpg", "jpeg"]);
        match dialog.pick_file() {
            Some(path) => self.add_image(&path),
            None => self.say_info("nothing chosen."),
        }
    }

    /// Decode a picture file and arm it, waiting for a click to place it at.
    ///
    /// The same decode this reads a signature with — see
    /// [`Self::upload_signature`] — minus the parts that are specific to a
    /// signature: no search for pen ink in a photo, and no naming it into the
    /// signature list. This is a plain picture, placed once, at whatever size
    /// and position somebody chose on the page — not kept anywhere to place
    /// again.
    fn add_image(&mut self, path: &std::path::Path) {
        if self.tab_mut().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.say_error(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        let not_a_picture = |e: image::ImageError| {
            format!("{} is not a picture this reads (PNG or JPEG): {e}", path.display())
        };
        let reader = match image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format() {
            Ok(reader) => reader,
            Err(e) => {
                self.say_error(not_a_picture(e.into()));
                return;
            }
        };
        let mut decoder = match reader.into_decoder() {
            Ok(decoder) => decoder,
            Err(e) => {
                self.say_error(not_a_picture(e));
                return;
            }
        };
        // Checked from the header, before a single pixel is decoded — see the
        // matching comment on `upload_signature`.
        let (declared_width, declared_height) = image::ImageDecoder::dimensions(&decoder);
        if pdf_core::render::bitmap::validate_dimensions(declared_width, declared_height).is_err()
        {
            self.say_error(format!(
                "{} is {declared_width}x{declared_height} — too large a picture to read.",
                path.display()
            ));
            return;
        }
        let orientation = image::ImageDecoder::orientation(&mut decoder)
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut decoded = match image::DynamicImage::from_decoder(decoder) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.say_error(not_a_picture(e));
                return;
            }
        };
        decoded.apply_orientation(orientation);
        let photo = decoded.to_rgba8();
        let (width, height) = photo.dimensions();

        let page = self.tab_mut().page;
        self.arm(Tool::PlaceImage { rgba: photo.into_raw(), width, height }, page);
    }

    /// Default width a placed picture gets on the page, in points — about two
    /// inches, before the click point's aspect ratio sets its height.
    const PLACED_IMAGE_WIDTH_PT: f32 = 200.0;

    /// Put a picture on the page, centred on `at`.
    ///
    /// Centred rather than anchored by a corner: unlike a signature, which
    /// sits *on* a line somebody clicked, a plain picture has no line to sit
    /// on — the click is only ever "about here", and centring it is the one
    /// choice that does not also silently pick a corner to grow from.
    fn place_image_at(
        &mut self,
        page: usize,
        at: AppPoint,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Result<String, String> {
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        let aspect = width as f32 / (height.max(1) as f32);
        let w = Self::PLACED_IMAGE_WIDTH_PT;
        let h = w / aspect.max(f32::EPSILON);
        let rect = pdf_core::document::Rect {
            left: at.x as f32 - w / 2.0,
            top: at.y as f32 - h / 2.0,
            right: at.x as f32 + w / 2.0,
            bottom: at.y as f32 + h / 2.0,
        };
        doc.session
            .execute(pdf_core::command::Command::AddAnnotation {
                page_index: page,
                annotation: pdf_core::document::Annotation::Image { rect, rgba, width, height },
            })
            .map_err(|e| format!("{e}"))?;

        // Without these two, a freshly placed picture drew from the stale
        // cached page texture until something unrelated happened to clear
        // it — showing up only several seconds later, by luck — and stayed
        // invisible to hit-testing until the page was left and returned to,
        // so it could not be selected, moved or resized. `place_signature`
        // already does both; a plain picture needs exactly the same.
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }

        Ok(format!("picture placed on page {}.", page + 1))
    }

    /// One page per row, or two as a spread.
    fn set_layout(&mut self, layout: pagify_shell::reader::Layout) {
        use pagify_shell::reader::Layout;
        // The strip is a different shape, so where the window was pointing no
        // longer means the same thing. Going back to the page the reader was on
        // is the only answer that survives the change.
        let page = self.tab().page;
        let Some(doc) = &mut self.tab_mut().doc else {
            self.say_error("nothing open.");
            return;
        };
        if let Ok(sizes) = doc.session.page_sizes() {
            // The turn the view is at survives a change of layout.
            doc.strip = Strip::with_layout_turned(&sizes, PAGE_GAP_PT, layout, doc.strip.turned());
        }
        let scroll_to_pt = doc.strip.top_of(page);
        self.tab_mut().scroll_to_pt = scroll_to_pt;
        self.tab_mut().settling = 3;

        self.say_info(match layout {
            Layout::Single => "one page at a time.",
            Layout::Facing => "two pages side by side.",
            Layout::FacingWithCover => "two pages side by side, the first one alone.",
        });
    }

    fn reverse_pages(&mut self) {
        let Some(doc) = &self.tab_mut().doc else {
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
        let Some(doc) = &self.tab_mut().doc else {
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
        let Some(doc) = &self.tab_mut().doc else {
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
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        self.say_info(said);
    }

    fn insert_page(&mut self) {
        let at = self.tab().page;
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return;
        };
        let (w, h) = self.tab().doc.as_ref().and_then(|d| d.strip.size_of(at)).unwrap_or((612.0, 792.0));
        let outcome = session.execute(pdf_core::command::Command::InsertBlankPage {
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
        let at = self.tab().page;
        self.import_at(source, spec, at)
    }

    /// Choose another PDF and bring all of its pages into this one — the Pages
    /// rail's own button for it, asked for from use: "have an option here to
    /// import pages from a different pdf".
    ///
    /// They land after the last page selected in the rail, or after the page
    /// being read when none is: the place somebody who has just clicked a page
    /// and asked for more pages means. The typed `import` command takes a page
    /// range and is the way to bring in only some of them.
    fn import_pages_dialog(&mut self) {
        if self.tab().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let dialog = rfd::FileDialog::new()
            .set_title("Insert pages from another PDF")
            .add_filter("PDF", &["pdf"]);
        match dialog.pick_file() {
            Some(path) => {
                let at = self.page_after_selection();
                self.import_at(&path, "all", at);
            }
            None => self.say_info("nothing chosen."),
        }
    }

    /// Where pages inserted "here" go: just after the last selected page, or
    /// just after the current one.
    fn page_after_selection(&self) -> usize {
        let last = self.tab().organize_selected.iter().copied().max().unwrap_or(self.tab().page);
        let count = self.tab().doc.as_ref().map_or(0, |d| d.page_count);
        (last + 1).min(count)
    }

    /// Bring pages `spec` of `source` into this document at `at`.
    fn import_at(&mut self, source: &std::path::Path, spec: &str, at: usize) {
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return;
        };
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

        match session.import_from(source, &pages, at) {
            Ok(total) => {
                self.say_info(format!("imported {} page(s); {total} in all.", pages.len()));
                self.refresh_after_page_change();
            }
            Err(e) => self.say_error(format!("import: {e}")),
        }
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

    /// The address a foreign-mark number from [`Self::foreign_at`] names,
    /// if it is a link — `None` for every other kind of mark.
    ///
    /// Re-reads `annotations(page)` rather than carrying the address
    /// through `foreign_marks`'s own cache: a link's rect is all hit-testing
    /// needs, and threading a second, mostly-unused field through a cache
    /// keyed for painting quad points would answer a question only a click
    /// asks.
    fn link_uri_at(&self, page: usize, n: usize) -> Option<String> {
        let marks = self.tab().doc.as_ref()?.session.annotations(page).ok()?;
        match &marks.get(n.checked_sub(1)?)?.annotation {
            pdf_core::document::Annotation::Link { uri, .. } => Some(uri.clone()),
            _ => None,
        }
    }

    /// Follow a link, or say why not — shared by a plain click and the
    /// right-click menu's own "Open" so the scheme restriction is written
    /// once. **Only ever a web address, and only ever `http`/`https`.** This
    /// program only ever writes those two schemes (see `apply_web_link`),
    /// but a link on the page did not have to come from here — any PDF
    /// somebody opens can carry a `/URI` action of its own, and handing an
    /// unexamined scheme straight to the OS launcher is how a `file://` or a
    /// UNC-style address ends up read by something that trusts it more than
    /// this click did.
    fn open_or_report_link(&mut self, n: usize, uri: &str) {
        if uri.starts_with("http://") || uri.starts_with("https://") {
            match open_in_browser(uri) {
                Ok(()) => self.say_info(format!("opening {uri}")),
                Err(e) => self.say_error(format!("could not open {uri}: {e}")),
            }
        } else {
            self.say_info(format!(
                "link {n} goes to \"{uri}\" — not a web address, so it was not opened."
            ));
        }
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
        let page = self.tab().page;
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return;
        };
        let marks = match session.annotations(page) {
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
        let page = self.tab().page;
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return;
        };
        let marks = match session.annotations(page) {
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
        let outcome = session.execute(pdf_core::command::Command::RemoveAnnotation {
            page_index: page,
            index: mark.index,
        });
        match outcome {
            Ok(_) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.say_info(format!("{what} removed — `undo` puts it back."));
            }
            Err(e) => self.say_error(format!("{e}")),
        }
    }

    /// Take the highlight, underline, strike-out or squiggle under a click off
    /// the page.
    ///
    /// **Reported from use: "there is no option for a user to erase a
    /// highlight; the eraser in the Draw tab should erase highlights too".**
    /// The Eraser only ever deleted what was *selected* among the drawn marks,
    /// and a highlight is a page annotation that is never selected, so nothing
    /// could reach it. One `RemoveAnnotation` — undoable like any other. Only
    /// the four text marks: a note, a link or a placed picture clicked with the
    /// Eraser is told what it is rather than quietly destroyed.
    fn erase_mark_at(&mut self, page: usize, at: AppPoint) -> Result<String, String> {
        use pdf_core::document::Annotation as A;
        let Some(number) = self.foreign_at(page, at) else {
            return Err("no highlight there — click on one to erase it.".into());
        };
        let index = number - 1;
        let what = {
            let doc = self.tab().doc.as_ref().ok_or_else(|| "nothing open.".to_string())?;
            let marks = doc.session.annotations(page).map_err(|e| e.to_string())?;
            match marks.get(index).map(|m| &m.annotation) {
                Some(A::Highlight { .. }) => "highlight",
                Some(A::Underline { .. }) => "underline",
                Some(A::StrikeOut { .. }) => "strike-out",
                Some(A::Squiggly { .. }) => "squiggle",
                Some(_) => return Err("that is not a highlight, underline or strike-out — the Eraser leaves it alone.".into()),
                None => return Err("no highlight there — click on one to erase it.".into()),
            }
        };
        self.remove_annotation_at(page, index)?;
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        Ok(format!("{what} erased — `undo` puts it back."))
    }

    /// Remove one annotation by its own engine index — a placed picture or
    /// signature, addressed the way [`Self::signature_at`]/
    /// [`Self::placed_image_at`] name them, not the position `marks` shows a
    /// reader. Shared by the Delete key's two annotation branches so the
    /// `session.execute`/error-mapping is written once.
    fn remove_annotation_at(&self, page: usize, index: usize) -> Result<(), String> {
        let doc = self.tab().doc.as_ref().ok_or_else(|| "nothing open.".to_string())?;
        doc.session
            .execute(pdf_core::command::Command::RemoveAnnotation { page_index: page, index })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// The Delete key, for whichever of the app's selection mechanisms is
    /// holding something right now.
    ///
    /// **Reported from use: a selected drawn shape or inserted picture could
    /// not be deleted.** A placed signature or plain picture is a bare
    /// annotation picked with no tool armed, and neither had ever been wired
    /// to Delete — only the object tool's own selection and the markup
    /// layer's had. Checked first here: the four selection mechanisms are
    /// mutually exclusive (see `Selected`, `SignatureSelected`,
    /// `PlacedImageSelected` and `Markup`'s own doc), so order between them
    /// only matters for reading, not behaviour. A free function of its own
    /// (rather than staying inline in `ui()`) so it can be called directly
    /// from a test without a real frame to drive `keys.delete` through.
    fn delete_selection(&mut self) {
        let page = self.tab_mut().page;
        if let Some(sel) = self.tab_mut().signature_selected.clone().filter(|s| s.page == page) {
            match self.remove_annotation_at(sel.page, sel.index) {
                Ok(()) => {
                    if let Some(doc) = &mut self.tab_mut().doc {
                        doc.rendered_is_stale();
                    }
                    self.tab_mut().signature_selected = None;
                    self.tab_mut().signature_grab = None;
                    self.say_info("signature removed — `undo` puts it back.");
                }
                Err(e) => self.say_error(e),
            }
            return;
        }
        if let Some(sel) = self.tab_mut().placed_image_selected.clone().filter(|s| s.page == page) {
            match self.remove_annotation_at(sel.page, sel.index) {
                Ok(()) => {
                    if let Some(doc) = &mut self.tab_mut().doc {
                        doc.rendered_is_stale();
                    }
                    self.tab_mut().placed_image_selected = None;
                    self.tab_mut().placed_image_grab = None;
                    self.say_info("picture removed — `undo` puts it back.");
                }
                Err(e) => self.say_error(e),
            }
            return;
        }
        // The object tool's own selection next — a picture, shape or run of
        // words picked with `editobject`, not the drawing layer
        // `erase_selection` below reaches. Falling through when there is no
        // object selected keeps today's behaviour for the drawing tools
        // exactly as it was.
        if !self.tab_mut().group.is_empty() {
            self.delete_group();
        } else if let Some(sel) = self.tab_mut().selected.clone() {
            let result = match &self.tab_mut().doc {
                Some(doc) => doc
                    .session
                    .execute(pdf_core::command::Command::RemoveObject {
                        page_index: sel.page,
                        object: sel.object,
                    })
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                None => Err("nothing open.".into()),
            };
            match result {
                Ok(()) => {
                    if let Some(doc) = &mut self.tab_mut().doc {
                        doc.rendered_is_stale();
                    }
                    self.tab_mut().selected = None;
                    self.say_info(format!("{} removed from page {}.", sel.what, sel.page + 1));
                }
                Err(e) => self.say_error(e),
            }
        } else if let Some(layer) = self.tab_mut().markup.existing_mut(page) {
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

    /// Mark the selected text — highlight, underline, strike out, squiggle.
    ///
    /// One annotation for the whole selection, however many lines it covers.
    /// A selection spanning three lines is one thing the reader made, so
    /// erasing it should be one action rather than three — which is why the
    /// engine's markup carries a *list* of rectangles.
    fn mark_selection(&mut self, kind: pagify_shell::verbs::Markup) {
        use pagify_shell::verbs::Markup;

        let Some(range) = self.tab_mut().text_selection.clone() else {
            // Nothing selected: pick the tool up rather than refuse. It stays
            // in hand until Escape or another tool, so a run of passages can be
            // marked without going back to the ribbon between each.
            self.put_down_page_editors("armed a markup tool");
            self.tab_mut().link_armed = false;
            self.tab_mut().match_properties_armed = false;
            self.tab_mut().match_properties_sample = None;
            self.tab_mut().markup_armed = Some(kind);
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
        let page = self.tab_mut().selection_page;

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

        let Some(doc) = &self.tab_mut().doc else {
            self.say_error("nothing open.");
            return;
        };
        match doc
            .session
            .execute(pdf_core::command::Command::AddAnnotation { page_index: page, annotation })
        {
            Ok(_) => {
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
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
        let page = self.tab().page;
        let Some(session) = self.tab().doc.as_ref().map(|d| d.session.clone()) else {
            self.say_error("nothing open.");
            return;
        };
        let rect = pdf_core::document::Rect { left: 24.0, top: 24.0, right: 44.0, bottom: 44.0 };
        match session.note(page, rect, text, MARKUP_INK) {
            Ok(_) => {
                self.say_info("note added at the top-left of the page.");
                if let Some(doc) = &mut self.tab_mut().doc {
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
                // **Into Pagify's own folder, under a name that is only a
                // name.** It used to be `<name>.json` relative to wherever the
                // process was started, with the name unchecked — so
                // `record ../../x` wrote `../../x.json`. Found by audit.
                let written = pagify_shell::state::file_name_only(&script.name)
                    .and_then(|name| {
                        self.scripts_dir
                            .as_ref()
                            .map(|dir| dir.join(format!("{name}.json")))
                            .ok_or_else(|| "there is nowhere to keep scripts on this system".into())
                    })
                    .and_then(|path| {
                        pagify_shell::state::write_own(&path, script.to_json().as_bytes())
                            .map(|()| path)
                            .map_err(|e| e.to_string())
                    });
                match written {
                    Ok(path) => self.say_info(format!(
                        "{} step(s) written to {}",
                        script.steps.len(),
                        path.display()
                    )),
                    Err(e) => self.say_error(format!("could not write the script: {e}")),
                }
            }
        }
    }

    /// Scripts nested this deep are not automation, they are a mistake — one
    /// that names itself, or two that name each other.
    const MAX_REPLAY_DEPTH: usize = 16;

    fn replay(&mut self, path: &std::path::Path) {
        if self.replay_depth >= Self::MAX_REPLAY_DEPTH {
            self.say_error(format!(
                "replay: {} scripts deep — a script is replaying itself, directly or through \
                 others. Stopped rather than recursing forever.",
                self.replay_depth
            ));
            return;
        }
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
        self.replay_depth += 1;
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
        self.replay_depth -= 1;
        match stopped {
            None => self.say_info(format!("replayed {ran} step(s).")),
            Some((step, line, why)) => {
                self.say_error(format!("stopped at step {step} (`{line}`): {why}. {ran} ran first."))
            }
        }
    }


    // -- rendering ----------------------------------------------------------

    /// The sharpest scale a *whole page* this size can be rendered at as one
    /// texture — the GPU's own largest-side limit, and a pixel-area budget of
    /// our own, well under the engine's hard ceiling (see [`Self::texture_for`]'s
    /// own doc for why that budget is sized the way it is). Pulled out of
    /// `texture_for` so [`Self::draw_detail_overlay`] can ask the same
    /// question — "is the whole-page texture actually enough here?" — without
    /// duplicating the arithmetic.
    fn whole_page_scale_ceiling(ctx: &egui::Context, w: f32, h: f32) -> f32 {
        let (w, h) = (w.max(1.0), h.max(1.0));
        // Brought down from 48M. The cost of a zoom step grows with the
        // pixels in the picture it makes — render, conversion and the upload to
        // the GPU — and at 48M (a 192 MB texture) the step itself was a stall
        // however it was scheduled. What the budget no longer covers is shown
        // sharp by the detail laid over the part being looked at, which is
        // millions of pixels and not tens of millions at any zoom.
        const BUDGET_PIXELS: f32 = 24.0 * 1024.0 * 1024.0;
        let max_side = (ctx.input(|i| i.max_texture_side) as f32)
            .min(pdf_core::render::bitmap::MAX_DIMENSION_PX as f32);

        let by_side = max_side / w.max(h);
        let by_area = (BUDGET_PIXELS / (w * h)).sqrt();
        by_side.min(by_area).max(0.05)
    }

    /// A page raster at `scale`, capped to what the GPU will hold.
    ///
    /// The cap lives here rather than at the call sites, and that is the point:
    /// it was at one of the two call sites, so the prefetch — which warms pages
    /// nobody is looking at yet — happily asked for a 3000-pixel texture and
    /// egui panicked. A limit every caller has to remember is a limit one of
    /// them will forget.
    ///
    /// **Reported from use**: a detail zoomed into on an A1 drawing looked
    /// visibly blurrier than the same file in another reader. This whole page
    /// is one texture, so a large-format sheet (A1, A0 — the common case for
    /// a floor plan or an RCP) hits [`Self::whole_page_scale_ceiling`] long
    /// before the GPU or the engine actually would, and everything past it is
    /// the GPU stretching that same under-resolved texture further, not a
    /// sharper render. [`Self::draw_detail_overlay`] is the real fix — it
    /// renders just the visible crop at the full scale instead, on top of
    /// whatever this function already drew — raising the budget here only
    /// pushes the point where that overlay kicks in a little further out.
    fn texture_for(&mut self, ctx: &egui::Context, page: usize, scale: f32) -> Option<egui::TextureHandle> {
        self.texture_or_request(ctx, page, scale, true)
    }

    /// Ask for a page's texture without waiting for it — the prefetch of the
    /// pages next to the one being read, which exists to be ready *before* it
    /// is looked at and so has no business stopping a frame.
    fn warm_texture(&mut self, ctx: &egui::Context, page: usize, scale: f32) {
        let _ = self.texture_or_request(ctx, page, scale, false);
    }

    /// Whether the zoom has moved within the last [`ZOOM_SETTLE_SECS`]. Asks for
    /// a frame at the moment it will have stopped, so the render it has been
    /// waiting for is not left until something else wakes the window.
    fn zoom_is_moving(&self, ctx: &egui::Context) -> bool {
        let since = ctx.input(|i| i.time) - self.tab().zoom_changed_at;
        if since < ZOOM_SETTLE_SECS {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(ZOOM_SETTLE_SECS - since + 0.01));
            true
        } else {
            false
        }
    }

    /// The pictures of pages that have come back from the render worker, put on
    /// screen — once a frame, on the UI thread, which is the only one that may
    /// make a texture.
    ///
    /// Anything that no longer describes the page is dropped here rather than
    /// shown: a render asked for before an edit, or for a document since closed.
    fn collect_renders(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.renders.as_mut() else { return };
        let mut arrived = Vec::new();
        while let Ok(done) = worker.done.try_recv() {
            // The slot belongs to the *newest* ask; an older answer arriving
            // must not release it.
            let slot = (done.doc, done.page, done.crop.is_some());
            if worker.in_flight.get(&slot) == Some(&(done.step, done.rotation as u8, done.crop)) {
                worker.in_flight.remove(&slot);
            }
            arrived.push(done);
        }
        for done in arrived {
            let Ok(image) = done.image else {
                self.render_stats.dropped += 1;
                if let Some(worker) = self.renders.as_mut() {
                    worker.failed.insert((done.doc, done.epoch, done.page, done.step, done.rotation as u8));
                }
                continue;
            };
            let current = self
                .tabs
                .iter_mut()
                .filter_map(|t| t.doc.as_mut())
                .find(|d| d.id == done.doc && d.render_epoch == done.epoch);
            let Some(doc) = current else {
                self.render_stats.dropped += 1;
                continue;
            };
            let [width, height] = image.size;
            let rotation = done.rotation as u8;
            match done.crop {
                None => {
                    let handle = ctx.load_texture(
                        format!("page{}", done.page),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    doc.caches.textures.insert((done.page, done.step, rotation), handle);
                    Self::trim_scales(doc, done.page, done.step, rotation);
                }
                Some(crop) => {
                    let texture = ctx.load_texture(
                        format!("detail{}", done.page),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    doc.caches.detail = Some(DetailTile { page: done.page, zoom_step: done.step, crop, texture });
                }
            }
            self.render_stats.applied += 1;
            self.render_stats.slowest_ms = self.render_stats.slowest_ms.max(done.took.as_millis());
            if done.took.as_millis() >= SLOW_RENDER_MS {
                self.session_log.record(
                    "perf",
                    &format!(
                        "page {}{} rendered off the UI thread in {} ms ({width}x{height} px)",
                        done.page + 1,
                        if done.crop.is_some() { " detail" } else { "" },
                        done.took.as_millis(),
                    ),
                );
            }
        }
    }

    /// Keep a page's pictures to [`SCALES_KEPT_PER_PAGE`]: the one just made and
    /// the nearest others.
    fn trim_scales(doc: &mut Doc, page: usize, step: u32, rotation: u8) {
        let held: Vec<u32> = doc
            .caches
            .textures
            .keys()
            .filter(|(p, _, r)| *p == page && *r == rotation)
            .map(|(_, s, _)| *s)
            .collect();
        for gone in scales_to_drop(&held, step, SCALES_KEPT_PER_PAGE) {
            doc.caches.textures.remove(&(page, gone, rotation));
        }
    }

    fn texture_or_request(
        &mut self,
        ctx: &egui::Context,
        page: usize,
        scale: f32,
        may_block: bool,
    ) -> Option<egui::TextureHandle> {
        let scale = {
            let (w, h) = self.tab_mut()
                .doc
                .as_ref()
                .and_then(|d| d.strip.size_of(page))
                .unwrap_or((612.0, 792.0));
            scale.min(Self::whole_page_scale_ceiling(ctx, w, h))
        };

        let rotation = self.tab().rotation;
        // **Quantised, not the raw continuous value.** A pinch or a
        // scroll-wheel zoom changes `scale` by a hair every single frame,
        // and keying on that directly — as this used to — made every one
        // of those frames a cache miss: a fresh `render_page_rotated` of
        // the whole page, plus a fresh GPU texture upload, none of it ever
        // reused even a moment later at what was functionally the same
        // zoom. `pdf_core::render::cache` already carries the exact fix for
        // this, `ZOOM_QUANTUM`/`quantise_zoom` (built for its own prefetch
        // cache, for the identical reason its own doc gives: "an
        // unquantised key would make every frame a miss while filling the
        // cache with near-duplicates") — reused here for the same problem
        // in the texture this app actually draws with, which never went
        // through that cache at all. Reported from use: zooming a real,
        // dense page felt "very laggy," constantly, independent of Edit
        // Text or anything else open at the time.
        let zoom_step = pdf_core::render::cache::quantise_zoom(scale);
        let key = (page, zoom_step, rotation as u8);
        // Rounds up, so a texture rendered to satisfy one scale in this
        // quantum band is never blurrier than any other scale in the same
        // band asks for — never above the ceiling just computed above,
        // which a rare, already-maxed-out zoom could otherwise round past.
        let quantised = (zoom_step as f32 * pdf_core::render::cache::ZOOM_QUANTUM).min(scale);

        let moving = self.async_render && self.zoom_is_moving(ctx);
        let async_render = self.async_render;
        let doc = self.tab_mut().doc.as_mut()?;

        if let Some(existing) = doc.caches.textures.get(&key) {
            return Some(existing.clone());
        }

        // **Not on this thread, if there is anything to show meanwhile.**
        // Reported from use: "zooming in and out on a pdf page is stuttery" —
        // every new step was a render right here, in the middle of a frame.
        // What is already held for this page, stretched to the size asked for,
        // is drawn instead; the right one is made by the worker and swapped in
        // when it comes back, and not started at all while the zoom is still
        // moving, because a picture for a step it has already left is time
        // taken from the one it stops on.
        if async_render {
            let stand_in = stand_in_for(doc, page, zoom_step, rotation as u8);
            if stand_in.is_some() || !may_block {
                // A page held only as its thumbnail is asked for at once: there is
                // no earlier picture of it for a zoom in motion to be drawn from.
                let only_a_thumbnail = stand_in.as_ref().is_some_and(|(_, thumbnail)| *thumbnail);
                if !moving || only_a_thumbnail {
                    let (doc_id, epoch, session) = (doc.id, doc.render_epoch, doc.session.clone());
                    self.ask_for_render(ctx, RenderJob {
                        doc: doc_id,
                        epoch,
                        page,
                        step: zoom_step,
                        rotation,
                        scale: quantised,
                        crop: None,
                        session,
                    });
                }
                return stand_in.map(|(texture, _)| texture);
            }
            // Nothing at all to draw yet — the first look at this page — so it
            // is rendered now, as it always was.
        }
        let doc = self.tab_mut().doc.as_mut()?;
        // A refusal must not mean a blank page. The ceilings above should make
        // this unreachable; if a limit is ever missed, a softer page is a far
        // better answer than no page.
        let mut scale = quantised;
        let started = std::time::Instant::now();
        let raster = loop {
            match doc.session.render_page_rotated(page, scale, rotation) {
                Ok(raster) => break raster,
                Err(_) if scale > 0.08 => scale *= 0.5,
                Err(_) => return None,
            }
        };
        let took = started.elapsed();
        let handle = ctx.load_texture(
            format!("page{page}"),
            page_to_image(&raster),
            egui::TextureOptions::LINEAR,
        );
        doc.caches.textures.insert(key, handle.clone());
        Self::trim_scales(doc, page, zoom_step, rotation as u8);
        self.render_stats.on_ui_thread += 1;
        // **What a stall was, written down where it can be read.** The session
        // log had no timing at all, so "it stutters" could not be answered
        // from it.
        if took.as_millis() >= SLOW_RENDER_MS {
            self.session_log.record(
                "perf",
                &format!(
                    "page {} rendered on the UI thread in {} ms ({}x{} px, {:.2}x) — a frame that long is a stall",
                    page + 1,
                    took.as_millis(),
                    raster.width,
                    raster.height,
                    scale
                ),
            );
        }
        Some(handle)
    }

    /// Hand a render to the worker — unless it is already working on exactly
    /// that. Starts the worker on the first ask.
    fn ask_for_render(&mut self, ctx: &egui::Context, job: RenderJob) {
        if self.renders.is_none() {
            self.renders = RenderWorker::start(ctx);
        }
        let ask = (job.doc, job.page, job.crop.is_some());
        let want = (job.step, job.rotation as u8, job.crop);
        let refused = (job.doc, job.epoch, job.page, job.step, job.rotation as u8);
        let Some(worker) = self.renders.as_mut() else {
            // No thread to render on: back to rendering where it is drawn,
            // rather than leaving a page soft for good.
            self.async_render = false;
            return;
        };
        if worker.in_flight.get(&ask) == Some(&want) || worker.failed.contains(&refused) {
            return;
        }
        if worker.jobs.send(job).is_ok() {
            worker.in_flight.insert(ask, want);
            self.render_stats.requested += 1;
        } else {
            // The worker has gone.
            self.renders = None;
            self.async_render = false;
        }
    }

    /// How many pixels across a thumbnail is rendered — the width it occupies on
    /// the *screen*, in physical pixels. A bitmap drawn at any other size is
    /// stretched or squeezed to fit, and looks it.
    fn thumb_width_px(logical_width: f32, pixels_per_point: f32) -> u32 {
        // Past the top a thumbnail is a page view, at megabytes apiece.
        (logical_width * pixels_per_point).round().clamp(32.0, 1024.0) as u32
    }

    /// A page's thumbnail, rendered `width_px` across.
    ///
    /// **Reported from use, with a screenshot: "the quality of the preview in
    /// the thumbnail is terrible".** It was always a fixed `0.12`-scale bitmap —
    /// about seventy pixels across — and the rail was changed to draw it as wide
    /// as the rail. Two to three times its own size is a blur, and a smoother
    /// filter only hides that; rendering at the size it is shown removes it.
    fn thumb_for(&mut self, ctx: &egui::Context, page: usize, width_px: u32) -> Option<egui::TextureHandle> {
        // While a button is down the rail may be mid-resize, and rendering every
        // visible page on every frame of a drag would stutter it: the bitmap
        // already held is stretched for the moment, and made sharp the instant
        // the pointer is released.
        let dragging = ctx.input(|i| i.pointer.any_down());
        let doc = self.tab_mut().doc.as_mut()?;
        if let Some(existing) = doc.caches.thumbs.get(&page) {
            let have = existing.size()[0] as u32;
            let right_size = have.abs_diff(width_px) <= 6;
            let usable_meanwhile = have * 2 >= width_px && have <= width_px * 2;
            if right_size || (dragging && usable_meanwhile) {
                return Some(existing.clone());
            }
        }
        let (width_pt, _) = doc.strip.size_of(page)?;
        let scale = width_px as f32 / width_pt.max(1.0);
        let raster = doc.session.render_page(page, scale).ok()?;
        let handle = ctx.load_texture(
            format!("thumb{page}"),
            page_to_image(&raster),
            // Mipmapped, so the stretch during a drag, and a bitmap a little
            // larger than it is shown, are averaged rather than aliased.
            egui::TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..egui::TextureOptions::LINEAR
            },
        );
        doc.caches.thumbs.insert(page, handle.clone());
        Some(handle)
    }

    /// How much further than what's actually visible a detail render covers,
    /// as a fraction of the visible size on each side — so a small pan or
    /// scroll does not immediately fall outside it and trigger another
    /// render. See [`Self::draw_detail_overlay`].
    const DETAIL_MARGIN_FRAC: f32 = 0.5;

    /// A screen rect through `view`, into page points, clamped to the page's
    /// own bounds — a crop past the page's own edge would otherwise reach
    /// the engine as an out-of-range render request. Pure and free of
    /// `self`/`egui::Ui` beyond the plain `view` value, so this is tested
    /// directly rather than only through a running UI.
    fn page_rect_from_screen(view: PageView, r: egui::Rect, w: f32, h: f32) -> pdf_core::document::Rect {
        let tl = view.to_page(r.min);
        let br = view.to_page(r.max);
        pdf_core::document::Rect {
            left: (tl.x as f32).clamp(0.0, w),
            top: (tl.y as f32).clamp(0.0, h),
            right: (br.x as f32).clamp(0.0, w),
            bottom: (br.y as f32).clamp(0.0, h),
        }
    }

    /// Whether a cached detail tile already covers what's visible now, at
    /// today's zoom — if so, [`Self::draw_detail_overlay`] can redraw it as
    /// is rather than asking the engine for a fresh render. Takes the tile's
    /// own fields rather than a `&DetailTile` so a test can call this
    /// without a real `egui::TextureHandle` to put in one; pure otherwise,
    /// same reasoning as [`Self::page_rect_from_screen`] just above.
    fn detail_tile_covers(
        tile_page: usize,
        tile_zoom_step: u32,
        tile_crop: pdf_core::document::Rect,
        page: usize,
        zoom_step: u32,
        visible_crop: pdf_core::document::Rect,
    ) -> bool {
        tile_page == page
            && tile_zoom_step == zoom_step
            && tile_crop.left <= visible_crop.left + 0.01
            && tile_crop.top <= visible_crop.top + 0.01
            && tile_crop.right >= visible_crop.right - 0.01
            && tile_crop.bottom >= visible_crop.bottom - 0.01
    }

}

/// A page's pixels as an egui image.
///
/// **Cheap for the pixels that are nearly all of them.** A page is rendered on
/// white, so every pixel is opaque and unmultiplied *is* premultiplied; the
/// general conversion does float arithmetic on every one, which at a few
/// thousand pixels a side is tens of milliseconds a zoom step spent on a
/// result that is the identity.
fn page_to_image(raster: &PageRaster) -> egui::ColorImage {
    let pixels = raster
        .pixels
        .chunks_exact(4)
        .map(|p| {
            if p[3] == 255 {
                egui::Color32::from_rgb(p[0], p[1], p[2])
            } else {
                egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])
            }
        })
        .collect();
    egui::ColorImage::new([raster.width as usize, raster.height as usize], pixels)
}

// ---------------------------------------------------------------------------
// Rendering pages off the UI thread
// ---------------------------------------------------------------------------

/// A page render handed to the render worker.
struct RenderJob {
    /// Which document — a result for one that has since been closed is dropped.
    doc: u64,
    /// The document's [`Doc::render_epoch`] when this was asked. Anything that
    /// changes what a page looks like moves it, and a render of the page as it
    /// *was* must never be put on screen.
    epoch: u64,
    page: usize,
    step: u32,
    rotation: Rotation,
    scale: f32,
    /// `None` for the whole page; a crop, in page points, for the sharper
    /// detail laid over a zoomed-in part of it — see
    /// [`PagifyApp::draw_detail_overlay`].
    crop: Option<pdf_core::document::Rect>,
    session: std::sync::Arc<Session>,
}

/// What the worker sends back — already an egui image, because turning a few
/// tens of millions of pixels into one is as slow as a small render and has no
/// more business on the UI thread.
struct RenderDone {
    doc: u64,
    epoch: u64,
    page: usize,
    step: u32,
    rotation: Rotation,
    crop: Option<pdf_core::document::Rect>,
    took: std::time::Duration,
    image: Result<egui::ColorImage, String>,
}

/// The thread that renders pages so the UI thread does not.
///
/// **Reported from use: "zooming in and out on a pdf page is stuttery".** Every
/// new zoom step was a page render on the UI thread — measured, 15–30 ms for an
/// ordinary page at ordinary zoom, 80–420 ms at high zoom, and 120–270 ms for a
/// drawing of sixty thousand shapes at *any* zoom — so a zoom was a run of
/// freezes, one per step. The frame now draws the nearest picture it already has
/// and this makes the right one in the meantime.
struct RenderWorker {
    jobs: std::sync::mpsc::Sender<RenderJob>,
    done: std::sync::mpsc::Receiver<RenderDone>,
    /// The latest ask per page — `(document, page, is a detail)` to
    /// `(step, rotation, crop)` — that has not come back. Only the newest
    /// matters, and the worker drops the older ones, so one slot per page and
    /// kind is the whole of what is outstanding.
    in_flight: HashMap<(u64, usize, bool), (u32, u8, Option<pdf_core::document::Rect>)>,
    /// Renders that came back refused, as `(document, epoch, page, step,
    /// rotation)` — not asked for again, or a page that cannot be drawn would be
    /// asked for on every frame for as long as it was on screen. A change to the
    /// page moves the epoch and so forgets them.
    failed: std::collections::HashSet<(u64, u64, usize, u32, u8)>,
}

impl RenderWorker {
    fn start(ctx: &egui::Context) -> Option<Self> {
        let (jobs, queue) = std::sync::mpsc::channel::<RenderJob>();
        let (reply, done) = std::sync::mpsc::channel::<RenderDone>();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pagify-render".into())
            .spawn(move || {
                while let Ok(first) = queue.recv() {
                    // Everything asked for while the last render ran. A zoom asks
                    // for a new size faster than a heavy page renders, and only the
                    // newest ask for a page is worth the time it takes.
                    let mut batch = vec![first];
                    batch.extend(queue.try_iter());
                    let mut newest: Vec<RenderJob> = Vec::new();
                    for job in batch.into_iter().rev() {
                        let same = |n: &RenderJob| {
                            n.doc == job.doc && n.page == job.page && n.crop.is_some() == job.crop.is_some()
                        };
                        if !newest.iter().any(same) {
                            newest.push(job);
                        }
                    }
                    for job in newest {
                        let started = std::time::Instant::now();
                        // A refusal must not mean a blank page: smaller, as the
                        // render on the UI thread always did, until it is not
                        // refused.
                        let mut scale = job.scale;
                        let raster = loop {
                            let attempt = match job.crop {
                                None => job.session.render_page_rotated(job.page, scale, job.rotation),
                                Some(crop) => job.session.render_page_region(job.page, crop, scale),
                            };
                            match attempt {
                                Err(_) if scale > 0.08 => scale *= 0.5,
                                other => break other,
                            }
                        };
                        let image = raster.map(|r| page_to_image(&r)).map_err(|e| e.to_string());
                        let sent = reply.send(RenderDone {
                            doc: job.doc,
                            epoch: job.epoch,
                            page: job.page,
                            step: job.step,
                            rotation: job.rotation,
                            crop: job.crop,
                            took: started.elapsed(),
                            image,
                        });
                        if sent.is_err() {
                            return;
                        }
                        // The answer is only seen on the next frame, and nothing
                        // else may be asking for one.
                        ctx.request_repaint();
                    }
                }
            })
            .ok()?;
        Some(RenderWorker { jobs, done, in_flight: HashMap::new(), failed: Default::default() })
    }
}

/// Counters for how pages were rendered — read by tests, and by the session log
/// when something was slow.
#[derive(Default, Debug, Clone, Copy)]
struct RenderStats {
    /// Rendered on the UI thread, because there was nothing to show meanwhile.
    on_ui_thread: u64,
    /// The same, for the detail laid over a zoomed-in part of a page.
    detail_on_ui_thread: u64,
    /// Asked of the worker.
    requested: u64,
    /// Came back and went on screen.
    applied: u64,
    /// Came back too late — the page had changed, or the document was gone.
    dropped: u64,
    /// The longest a render took in the worker, in milliseconds.
    slowest_ms: u128,
}

/// How long the zoom must hold still before a new picture is rendered for it.
///
/// A wheel or a pinch moves the zoom every few frames, and a render started for
/// a step the zoom has already left is time taken from the one it ends on.
const ZOOM_SETTLE_SECS: f64 = 0.12;

/// A render slower than this is written to the session log.
const SLOW_RENDER_MS: u128 = 40;

/// How many pictures of one page are kept at different scales. A zoom through a
/// dozen steps otherwise leaves a dozen full-page textures on the GPU for as
/// long as the page is on screen — each of them megabytes, the last few of them
/// hundreds.
const SCALES_KEPT_PER_PAGE: usize = 3;

/// Which of a page's pictures to throw away: all but the `keep` nearest in scale
/// to `step`. Pure, so it can be checked without a GPU.
fn scales_to_drop(held: &[u32], step: u32, keep: usize) -> Vec<u32> {
    let mut by_distance: Vec<u32> = held.to_vec();
    by_distance.sort_by_key(|s| (s.abs_diff(step), *s));
    by_distance.split_off(keep.min(by_distance.len()))
}

/// The picture to draw while the right one is being made: the one this page
/// already has nearest in scale — preferring a sharper one to a softer, which a
/// zoom-in would otherwise stretch — or failing that its thumbnail.
///
/// The flag says it *is* the thumbnail: not a picture of the page at some other
/// zoom but the little one from the rail, which a render should not wait out the
/// zoom's settling for.
fn stand_in_for(doc: &Doc, page: usize, step: u32, rotation: u8) -> Option<(egui::TextureHandle, bool)> {
    let nearest = doc
        .caches
        .textures
        .iter()
        .filter(|((p, _, r), _)| *p == page && *r == rotation)
        // A softer picture counts double: stretching one up is what shows.
        .min_by_key(|((_, s, _), _)| if *s >= step { *s - step } else { (step - *s) * 2 })
        .map(|(_, texture)| texture.clone());
    if let Some(texture) = nearest {
        return Some((texture, false));
    }
    // The thumbnail is of the page upright, so only a view that is upright.
    if rotation == 0 {
        return doc.caches.thumbs.get(&page).cloned().map(|t| (t, true));
    }
    None
}

static NEXT_DOC_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl eframe::App for PagifyApp {
    /// Run before every `ui`, **and on its own while the window is minimised**,
    /// when eframe runs no egui pass at all. That second case is why a document
    /// handed over by another launch is taken here and not in `ui`: measured, a
    /// minimised window never answered, the launch gave up waiting, and Pagify
    /// opened a second window — the one thing it exists to prevent. See
    /// `instance`.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.take_handover(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let command_id = self.command_id();

        self.draw_frame_preamble(&ctx);
        self.handle_input(&ctx, frame, command_id);
        self.open_dropped_files(&ctx);
        self.draw_title_bar(&ctx, ui);
        let ribbon_command = self.draw_ribbon(ui);

        let mut submitted = None;
        self.draw_command_bar(ui, command_id, &mut submitted);
        self.draw_main_area(&ctx, ui, command_id, ribbon_command, &mut submitted);
    }

}

impl PagifyApp {


    /// A ghost of the currently-chosen signature at `at` — the exact size
    /// and anchor [`Self::place_signature`] would actually use (see
    /// [`SIGNATURE_WIDTH_PT`]), so hovering the tool over the page shows how
    /// much of it the signature would cover, and where, before it is
    /// placed rather than after. Deliberately not [`paint_signature`],
    /// which pads and centres within whatever box it is given for a tidy
    /// thumbnail in the signature list — exactly what an accurate preview
    /// must not do.
    fn draw_signature_preview(&mut self, ui: &mut egui::Ui, view: PageView, at: AppPoint) {
        let Some(signature) = self.signatures.current().cloned() else { return };
        let rect = signature.placed_rect(at.x as f32, at.y as f32, SIGNATURE_WIDTH_PT);
        let area = egui::Rect::from_min_max(
            view.to_screen(AppPoint::new(rect.left as f64, rect.top as f64)),
            view.to_screen(AppPoint::new(rect.right as f64, rect.bottom as f64)),
        );

        if let Some(image) = signature.image.clone() {
            let texture = self
                .signature_textures
                .entry(signature.name.clone())
                .or_insert_with(|| {
                    ui.ctx().load_texture(
                        format!("signature-{}", signature.name),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [image.width as usize, image.height as usize],
                            &image.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                })
                .clone();
            // Semi-transparent — a preview, not yet a mark on the page.
            ui.painter().image(
                texture.id(),
                area,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::from_white_alpha(190),
            );
        } else {
            let ink = theme::violet().gamma_multiply(0.85);
            for stroke in signature.placed(at.x as f32, at.y as f32, SIGNATURE_WIDTH_PT) {
                if stroke.len() < 2 {
                    continue;
                }
                let points: Vec<egui::Pos2> =
                    stroke.iter().map(|(x, y)| view.to_screen(AppPoint::new(*x as f64, *y as f64))).collect();
                ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.8, ink)));
            }
        }

        // The footprint, so the size and shape read at a glance even before
        // the ink or the picture itself does.
        ui.painter().rect_stroke(
            area,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, theme::violet_bright()),
            egui::StrokeKind::Outside,
        );
    }

    /// The editor for a run of text, drawn over the words themselves.
    ///
    /// Sized and placed from the run's own box, so what is being typed sits
    /// where what is being replaced sits. A field somewhere else would mean
    /// holding the page in your head while you type.
    fn draw_run_editor(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        // Read once, before the borrow below — app-wide font state, not any
        // one paragraph's own.
        let em_ratio = self.editor_face_metrics.map(|m| (m.ascent - m.descent) as f32 / 1000.0);
        let face_ready = self.editor_face.is_some() && self.editor_face_ready;
        let coverage = self.editor_face_coverage.clone();

        let Some(edit) = self.tab_mut().editing_run.as_mut() else { return };
        if edit.page != page {
            return;
        }

        let top_left = view.to_screen(AppPoint {
            x: edit.rect.left.min(edit.rect.right) as f64,
            y: edit.rect.top.min(edit.rect.bottom) as f64,
        });
        let bottom_right = view.to_screen(AppPoint {
            x: edit.rect.left.max(edit.rect.right) as f64,
            y: edit.rect.top.max(edit.rect.bottom) as f64,
        });

        // **Exactly the run's own size, not padded to leave room to grow.**
        //
        // This used to add a flat 80 screen pixels (and a 120px floor) to
        // the width, reasoning that a replacement is rarely the same length
        // as what it replaces. That made the box itself lie about how big
        // the text actually is — reported from use as having "no idea,
        // relative to the text I am editing, how the new one will land".
        // `egui::TextEdit` does not clip text past its own rect; a longer
        // replacement is still fully visible, just no longer inside a box
        // that was pre-stretched to guess how long it might be.
        // **The box itself grows and shrinks with the Size slider.**
        //
        // Reported from use: "it locked in a text box, so i cant see the
        // actual scale it will be once i increase the font size" — the font
        // drawn inside already tracked `edit.style.size` live (see
        // `on_screen` below), but the box around it stayed pinned to
        // whatever screen rectangle the *original* run measured, so a bigger
        // size just crowded or overflowed the same fixed frame instead of
        // visibly growing. `grow` is how much bigger the current size draws
        // than the size the run opened at, and both dimensions of the box
        // scale by exactly that, so the box is always showing the real
        // footprint of whatever is being typed right now, not what fit
        // before the edit started.
        //
        // Anchored at bottom-left rather than top-left: a font's own anchor
        // is its baseline, near the bottom of the box, so growing upward and
        // rightward from there reads as the text getting bigger in place
        // rather than the box drifting away from where the words actually
        // sit.
        //
        // **Floored at a bare epsilon, not a readable pixel size.** This used
        // to be `.max(14.0)` — a floor meant to guard the arithmetic below
        // against a literally empty rect, not to keep the box a minimum
        // *readable* size. But it did exactly the latter too: a page zoomed
        // out enough for this run's own true on-screen height to fall under
        // 14 pixels — an ordinary "zoomed out to see the page" level, not an
        // extreme one — had that height propped back up while the rest of
        // the page kept shrinking normally, so the editor visibly grew
        // relative to the page the further out the zoom went. Reported from
        // use, twice, as the preview's own scale changing with zoom; the
        // same fix `run_editor_glyph_size` already needed for the font
        // drawn inside this box, one level up, for the box itself.
        let base_screen_height = (bottom_right.y - top_left.y).max(0.5);
        let base_screen_width = (bottom_right.x - top_left.x).max(0.5);
        let base_on_screen = run_editor_font_size(
            base_screen_height,
            edit.lines.len(),
            edit.was.size.unwrap_or(0.0),
            view.scale,
            em_ratio,
        );
        let on_screen = run_editor_font_size(
            base_screen_height,
            edit.lines.len(),
            edit.style.size.unwrap_or(0.0),
            view.scale,
            em_ratio,
        );
        let grow = run_editor_box_grow(base_on_screen, on_screen);
        let height = base_screen_height * grow;

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
        let (paper, ink, font_id, glyph_size) = Self::run_editor_skin(edit, on_screen, face_ready);
        let id = egui::Id::new(("run-editor", page, edit.object));

        // **Typing past the run's own edge starts a new line, rather than
        // growing the box sideways — but only for a single run.** Only the
        // buffer's own last line — the text after its final `\n`, wherever
        // that is — is ever measured or touched here, which is exactly
        // right when there is only one line to ever extend, but is not
        // "whichever line the person is actually typing" once a paragraph
        // has several: `edit.lines` maps `typed.split('\n')` to this
        // paragraph's own objects *by position* (see `apply_paragraph_edit`'s
        // own doc), and a `\n` inserted anywhere other than the true end
        // shifts every line after it by one, misrouting each remaining
        // line's text onto the next structural object and pushing the
        // paragraph's own last line into `write_extra_styled_lines` as a
        // stray, differently-styled extra. Reported from use: editing a
        // multi-line paragraph, then deleting text from the end — which
        // collapses the buffer until an *edited, interior* line becomes its
        // new last segment — corrupted the page: a word cut off mid-way, a
        // different font, and a wrong-coloured duplicate of a word left
        // behind. That failure mode cannot arise for a single run, where
        // everything past line 0 already goes to `write_extra_styled_lines`
        // on purpose (see `apply_one_edit`'s own `extra_lines`) regardless
        // of how many wraps happen — there is no earlier structural line for
        // a wrap to misroute onto.
        //
        // **Broken at the last word gap that still fits**, the same
        // `split_inclusive(' ')` tokenising `justify`'s own gap computation
        // below already trusts. A line with nothing to break at — one word
        // wider than the whole box — is left to overflow rather than cut a
        // word in half; `width`, just below, still grows to show it rather
        // than clipping it silently.
        //
        // ponytail: the newline lands correctly in the buffer, but egui's
        // own cursor state is not shifted to match, so the caret can appear
        // one character behind the words just typed until the next
        // keystroke resyncs it. Bumping the stored `CCursorRange` by one for
        // every inserted `\n` would close that gap, if it is ever reported.
        //
        // **Floored at the original line's own rendered width, not just the
        // run's geometric one.** The editor's own substitute face does not
        // always match the document's glyph widths character for character
        // — see `text_width`, just below, for the same fact — so the run's
        // own *unedited* words can already measure wider here than
        // `base_screen_width` reports. Using the geometric width alone
        // wrapped a pristine, freshly opened run before anyone had typed
        // anything: reported by a test failing the moment a run was picked,
        // not once anything was retyped. Measuring however wide the
        // original words already need in this font, once, keeps that case
        // silent — wrapping only kicks in once typing pushes *past* that.
        let max_width =
            Self::run_editor_max_width(edit, ui, &font_id, ink, base_screen_width, grow, view.scale);
        // Resized since last frame: the lines are joined, then wrapped afresh to
        // the new width by the loop below. One byte each way (`\n` for ` `), so
        // the caret does not move.
        if std::mem::take(&mut edit.box_resize.reflow) && edit.lines.len() <= 1 {
            edit.buffer = edit.buffer.replace('\n', " ");
        }
        Self::wrap_typed_last_line(edit, ui, &font_id, ink, max_width);

        // **Wide enough for what is actually typed, not just what the run
        // started at.** Ordinarily `max_width` above already keeps every
        // line within it — this only still grows the box for the one case
        // just above that deliberately overflows instead of cutting a word
        // in half. Reported from use, before wrapping existed at all, as
        // text cut off at the box's own edge.
        let text_width = Self::run_editor_text_width(edit, ui, &font_id, ink);
        let width = max_width.max(text_width + 8.0);
        // **As tall as the lines it holds.** The first line stays where the run
        // is and the others fall below it, so the box grows downwards — it used
        // to stay one line tall, with the wrapped lines hanging out of its
        // bottom.
        let extra_lines = edit.buffer.split('\n').count().saturating_sub(1);
        let row_height = ui.fonts_mut(|f| f.row_height(&font_id));
        let rect = egui::Rect::from_min_size(
            egui::pos2(top_left.x + edit.box_resize.left_shift_pt * view.scale, bottom_right.y - height),
            egui::vec2(width, height + 6.0 + extra_lines as f32 * row_height),
        );
        ui.painter().rect_filled(rect.expand(1.0), 0.0, paper);
        // **Reported from use**: clicking different parts of a page picked up
        // different, sometimes surprising spans of text to edit, with
        // nothing on screen showing the span until typing was already
        // underway. `draw_new_text_box` already draws exactly this — a
        // bordered box around what is about to change — for a brand new text
        // box; the run/paragraph editor here never did. Same border, same
        // reason: see the scope before committing to it, not after.
        ui.painter().rect_stroke(
            rect.expand(1.0),
            0.0,
            egui::Stroke::new(1.0, theme::violet_bright()),
            egui::StrokeKind::Outside,
        );

        // **Justified, the same as the page it came from — real
        // justification, not left-aligned and ragged.** `egui::TextEdit`
        // has no setting for this; a custom layouter builds the paragraph's
        // `LayoutJob` by hand instead, one line at a time, so it can insert
        // exactly the extra space `justify_gaps` computes for that line
        // rather than leaving every gap at its natural width. See that
        // function's own doc for why `LayoutJob::justify` itself is not
        // enough here.
        //
        // **Only for a run that already had more than one line.** A single
        // heading growing a second line of its own (see `apply_one_edit`)
        // is not a page's own justified paragraph reflowing — it is new
        // words with nothing to stretch to fill, and stretching them anyway
        // would be inventing a look the page never had.
        //
        // The paragraph's own last line is excluded, matching ordinary
        // typesetting — a short closing line is not stretched to fill the
        // column just because the lines above it were. See
        // `paragraph_should_justify`'s own doc for why "more than one line"
        // alone is not enough.
        let justify = paragraph_should_justify(&edit.lines);
        let format = egui::TextFormat { font_id: font_id.clone(), color: ink, ..Default::default() };
        // **What the document's own face has no ink for is drawn in the
        // program's own face instead** — same size, same colour. The family
        // already lists a fallback behind the document's face, but egui only
        // reaches it for a character the face lacks *entirely*; a subset that
        // keeps the letter's `cmap` entry and advance but emptied its outline
        // is found, drawn blank, and never fallen back from. See
        // `editor_sections`. Only when the document's face is the one in use:
        // before it is installed the whole box is already the program's own.
        let fallback = egui::TextFormat {
            font_id: egui::FontId::proportional(glyph_size),
            color: ink,
            ..Default::default()
        };
        let coverage = if face_ready { coverage } else { None };
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            // Everything is drawable until the face's coverage is known.
            let covered = |c: char| coverage.as_ref().map_or(true, |known| known.has(c));
            let job = editor_layout_job(
                buf.as_str(),
                justify,
                wrap_width,
                &format,
                &fallback,
                &covered,
                &mut |word: &str| {
                    // The same two-face layout the job gets, so a stretched
                    // line's measured words are as wide as they will be drawn.
                    let mut measured = egui::text::LayoutJob::default();
                    append_sectioned(&mut measured, word, 0.0, &format, &fallback, &covered);
                    ui.fonts_mut(|f| f.layout_job(measured)).size().x
                },
            );
            ui.fonts_mut(|f| f.layout_job(job))
        };

        {
            let style = ui.style_mut();
            style.visuals.override_text_color = Some(ink);
            style.visuals.extreme_bg_color = paper;
            style.visuals.selection.bg_fill = theme::violet().gamma_multiply(0.35);
            // Always multiline: a single run can grow a second line of its
            // own exactly the way a paragraph already could (see
            // `apply_one_edit`), so Enter has to add a line here too
            // rather than submit — a singleline field could never do that.
            let editor = egui::TextEdit::multiline(&mut edit.buffer).layouter(&mut layouter);
            // `.frame(Frame::NONE)`, not just a background colour and zero
            // margin: leaving the frame unset made egui fall back to its own
            // ordinary widget chrome underneath — this app's rounded corners
            // and, once focused, a violet selection stroke — painted right on
            // top of the flat paper fill this function already draws by
            // hand. A border and a rounded corner where the real page has
            // neither is exactly the boxed, generic-editor look this is
            // meant to avoid; a blank frame leaves only what is drawn
            // explicitly.
            let response = ui.put(rect, editor.id(id).frame(egui::Frame::NONE));
            // Once, the moment this editor first appears — not every frame,
            // or typing would never be able to give the focus back to
            // whatever else the user clicked afterwards.
            if !edit.focused {
                edit.focused = true;
                response.request_focus();
            }
        }
        ui.style_mut().visuals.override_text_color = None;

        // **Handles on the left and right edges: the box is a width to wrap the
        // words to**, so the same words can sit on one line or several. Declared
        // after the page's own response, so a press on one is the handle's and
        // not a click away from the editor. Single runs only: a paragraph's
        // lines are its page's own, not a width to refold (see `RunBox`).
        if edit.lines.len() <= 1 && !edit.drawn {
            let scale = view.scale.max(0.01);
            for handle in [Handle::Left, Handle::Right] {
                let at = egui::pos2(
                    if handle == Handle::Left { rect.left() } else { rect.right() },
                    rect.center().y,
                );
                let grip = ui.interact(
                    egui::Rect::from_center_size(at, egui::vec2(14.0, 18.0)),
                    id.with(("grip", handle as u8)),
                    egui::Sense::drag(),
                );
                if grip.hovered() || grip.dragged() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
                }
                if grip.dragged() {
                    let current = rect.width() / scale;
                    let dragged = grip.drag_delta().x / scale;
                    let wanted = if handle == Handle::Left { current - dragged } else { current + dragged };
                    let width = wanted.max(MIN_RUN_BOX_PT);
                    let applied = if handle == Handle::Left { current - width } else { width - current };
                    if applied.abs() > f32::EPSILON {
                        edit.box_resize.width_pt = Some(width);
                        if handle == Handle::Left {
                            edit.box_resize.left_shift_pt += applied;
                        }
                        edit.box_resize.reflow = true;
                        ui.ctx().request_repaint();
                    }
                }
                let square = egui::Rect::from_center_size(at, egui::vec2(HANDLE_PX * 2.0 + 1.0, HANDLE_PX * 2.0 + 1.0));
                ui.painter().rect_filled(square, 1.0, egui::Color32::WHITE);
                ui.painter().rect_stroke(
                    square,
                    1.0,
                    egui::Stroke::new(1.5, theme::violet_bright()),
                    egui::StrokeKind::Outside,
                );
            }
        }

        // No hairline, no grip: matches the look of the page around it
        // rather than marking itself off as a widget.

        // **Enter never submits here — only Apply does**, for a single line
        // exactly the same as it already does for a paragraph. A paragraph's
        // own multiline box already had to treat Enter as "add a line", not
        // "submit", since a paragraph legitimately grows a line; a single
        // line used to be the odd one out, submitting immediately, which
        // read as a change going live before anyone pressed the button that
        // says "apply this" — reported from use. Escape still leaves
        // whatever was typed here alone.
    }


/// Wrap the editor buffer's own last line at the widest word gap that still
/// fits `max_width`, up to 64 times. Correct only for a single-run editor —
/// see the full reasoning in `draw_run_editor`'s own comments — and kept
/// here so the drawing function is about the box, not the buffer.
fn wrap_typed_last_line(
    edit: &mut EditingRun,
    ui: &mut egui::Ui,
    font_id: &egui::FontId,
    ink: egui::Color32,
    max_width: f32,
) {
        if edit.lines.len() <= 1 {
            for _ in 0..64 {
                let last_start = edit.buffer.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let last_line = &edit.buffer[last_start..];
                let last_width = ui
                    .fonts_mut(|f| f.layout_no_wrap(last_line.to_string(), font_id.clone(), ink))
                    .size()
                    .x;
                if last_width <= max_width {
                    break;
                }
                let tokens: Vec<&str> = last_line.split_inclusive(' ').collect();
                if tokens.len() < 2 {
                    break;
                }
                let mut acc_len = 0usize;
                let mut break_at = None;
                for (i, token) in tokens.iter().enumerate() {
                    let candidate = &last_line[..acc_len + token.len()];
                    let candidate_width = ui
                        .fonts_mut(|f| {
                            f.layout_no_wrap(candidate.trim_end().to_string(), font_id.clone(), ink)
                        })
                        .size()
                        .x;
                    if candidate_width > max_width && i > 0 {
                        break_at = Some(acc_len);
                        break;
                    }
                    acc_len += token.len();
                }
                let Some(break_at) = break_at else { break };
                let insert_at = last_start + break_at;
                if edit.buffer.as_bytes().get(insert_at.wrapping_sub(1)) == Some(&b' ') {
                    edit.buffer.replace_range(insert_at - 1..insert_at, "\n");
                } else {
                    edit.buffer.insert(insert_at, '\n');
                }
            }
        }
}


    /// The skin values the run editor draws with: the page's own paper colour
    /// behind, the run's own ink in front, the font the buffer is set in — the
    /// run's embedded face where one is ready, the program's own proportional
    /// font otherwise — and the on-screen glyph size the fallback face uses.
    fn run_editor_skin(
        edit: &EditingRun,
        on_screen: f32,
        face_ready: bool,
    ) -> (egui::Color32, egui::Color32, egui::FontId, f32) {
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

        // See `run_editor_glyph_size`'s own doc for why this is not clamped
        // to a fixed pixel range — reported from use as the preview's own
        // scale changing as the page was zoomed in and out.
        let glyph_size = run_editor_glyph_size(on_screen);
        let font_id = if face_ready {
            egui::FontId::new(glyph_size, egui::FontFamily::Name(RUN_FAMILY.into()))
        } else {
            egui::FontId::proportional(glyph_size)
        };
        (paper, ink, font_id, glyph_size)
    }

    /// The widest of the buffer's own lines, measured as it will be drawn.
    fn run_editor_text_width(
        edit: &EditingRun,
        ui: &mut egui::Ui,
        font_id: &egui::FontId,
        ink: egui::Color32,
    ) -> f32 {
        edit.buffer
            .split('\n')
            .map(|line| ui.fonts_mut(|f| f.layout_no_wrap(line.to_string(), font_id.clone(), ink)).size().x)
            .fold(0.0f32, f32::max)
    }

    /// How wide the box is allowed to get: an explicit resize wins, otherwise
    /// the run's own measured width — see the comments this moved away from
    /// in `draw_run_editor` for why it grows with the size slider.
    fn run_editor_max_width(
        edit: &EditingRun,
        ui: &mut egui::Ui,
        font_id: &egui::FontId,
        ink: egui::Color32,
        base_screen_width: f32,
        grow: f32,
        view_scale: f32,
    ) -> f32 {
        let original_last_line = edit.original.rsplit('\n').next().unwrap_or("");
        let original_width = ui
            .fonts_mut(|f| f.layout_no_wrap(original_last_line.to_string(), font_id.clone(), ink))
            .size()
            .x;
        // A width the reader dragged the box to is the width, exactly: no
        // floor at what the unedited words measured, or it could never be
        // dragged narrower than the run it opened on.
        let max_width = match edit.box_resize.width_pt {
            Some(width_pt) => (width_pt * view_scale).max(1.0),
            None => (base_screen_width * grow).max(original_width),
        };
        max_width
    }



    /// The box [`Self::begin_text_box`] opened, being typed into.
    ///
    /// Auto-wraps to the box's own width, unlike the run editor's paragraph
    /// box above — this is new content with no existing lines to keep in
    /// place, so wrapping to whatever width was dragged out is exactly what
    /// "click and drag the area it should be in" asked for. The wrap egui
    /// does here on screen is reproduced exactly in
    /// [`Self::apply_new_text_box`], from the same width and font size, so
    /// what gets written matches what was typed.
    fn draw_new_text_box(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let Some(new_text) = &mut self.tab_mut().new_text_box else { return };
        if new_text.page != page {
            return;
        }

        let top_left = view.to_screen(AppPoint {
            x: new_text.rect.left as f64,
            y: new_text.rect.top as f64,
        });
        let bottom_right = view.to_screen(AppPoint {
            x: new_text.rect.right as f64,
            y: new_text.rect.bottom as f64,
        });
        let rect = egui::Rect::from_min_max(top_left, bottom_right);
        let paper = egui::Color32::from_rgb(250, 250, 248);

        ui.painter().rect_filled(rect.expand(1.0), 0.0, paper);
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, theme::violet_bright()),
            egui::StrokeKind::Outside,
        );

        let size_px = (new_text.size * view.scale).clamp(6.0, 200.0);
        let id = egui::Id::new(("new-text-box", page));
        let response = ui.put(
            rect,
            egui::TextEdit::multiline(&mut new_text.buffer)
                .id(id)
                .background_color(paper)
                .margin(egui::Margin::same(2))
                .font(egui::FontId::proportional(size_px)),
        );

        if !new_text.focused {
            new_text.focused = true;
            response.request_focus();
        }

        // The box can be dragged to any size from its eight handles, like a
        // picture picked with Edit Object. Its width is what the typing wraps
        // to, so the words re-wrap as it moves (see `apply_new_text_box`).
        let scale = view.scale.max(0.01);
        for handle in Handle::ALL {
            let (hx, hy) = handle.at(&new_text.rect);
            let at = view.to_screen(AppPoint { x: hx as f64, y: hy as f64 });
            let grip = ui.interact(
                egui::Rect::from_center_size(at, egui::vec2(14.0, 14.0)),
                id.with(("grip", handle as u8)),
                egui::Sense::drag(),
            );
            if grip.hovered() || grip.dragged() {
                ui.output_mut(|o| {
                    o.cursor_icon = match handle {
                        Handle::Left | Handle::Right => egui::CursorIcon::ResizeHorizontal,
                        Handle::Top | Handle::Bottom => egui::CursorIcon::ResizeVertical,
                        Handle::TopLeft | Handle::BottomRight => egui::CursorIcon::ResizeNwSe,
                        _ => egui::CursorIcon::ResizeNeSw,
                    }
                });
            }
            if grip.dragged() {
                let by = grip.drag_delta() / scale;
                let r = &mut new_text.rect;
                match handle {
                    Handle::Left | Handle::TopLeft | Handle::BottomLeft => {
                        r.left = (r.left + by.x).min(r.right - MIN_RUN_BOX_PT)
                    }
                    Handle::Right | Handle::TopRight | Handle::BottomRight => {
                        r.right = (r.right + by.x).max(r.left + MIN_RUN_BOX_PT)
                    }
                    _ => {}
                }
                match handle {
                    Handle::Top | Handle::TopLeft | Handle::TopRight => {
                        r.top = (r.top + by.y).min(r.bottom - MIN_RUN_BOX_PT)
                    }
                    Handle::Bottom | Handle::BottomLeft | Handle::BottomRight => {
                        r.bottom = (r.bottom + by.y).max(r.top + MIN_RUN_BOX_PT)
                    }
                    _ => {}
                }
                ui.ctx().request_repaint();
            }
            let square = egui::Rect::from_center_size(at, egui::vec2(HANDLE_PX * 2.0 + 1.0, HANDLE_PX * 2.0 + 1.0));
            ui.painter().rect_filled(square, 1.0, egui::Color32::WHITE);
            ui.painter().rect_stroke(
                square,
                1.0,
                egui::Stroke::new(1.5, theme::violet_bright()),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// The right-side panel a text run or a drawn shape's properties show
    /// in — replacing the run editor's old floating box, per the request
    /// that selecting something open a persistent panel rather than a
    /// ribbon that follows the selection around the page.
    ///
    /// A text run, a markup shape and a brand new text box are never two of
    /// the three at once — three separate mechanisms (`EditingRun`,
    /// `Layer::selection`, `NewTextBox`) that every tool arming itself
    /// already clears the others for — so this shows exactly one section,
    /// or nothing, rather than switching between tabs.
    fn draw_properties_panel(&mut self, ui: &mut egui::Ui) {
        // **Every frame, before anything is drawn from the editor:** words an
        // editor was let go of without being applied go to the clipboard (the
        // apply that let them go had no `egui::Context` to put them there with),
        // and an editor whose page changed under it is closed and says why — it
        // names objects by number, and nobody has to press Apply to be told.
        if let Some(text) = self.text_to_offer.take() {
            ui.ctx().copy_text(text);
        }
        self.close_editor_if_stale();
        if let Some(text) = self.text_to_offer.take() {
            ui.ctx().copy_text(text);
        }
        let text_page = self.tab_mut().editing_run.as_ref().map(|e| e.page);
        let new_text_page = self.tab_mut().new_text_box.as_ref().map(|b| b.page);
        let page = text_page.or(new_text_page).unwrap_or(self.tab_mut().page);
        let has_shapes = text_page.is_none()
            && new_text_page.is_none()
            && self.tab_mut().markup.existing(page).is_some_and(|l| !l.selection().is_empty());
        if text_page.is_none() && new_text_page.is_none() && !has_shapes {
            return;
        }

        egui::Panel::right("properties_panel")
            .resizable(true)
            .default_size(220.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::paper())
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Properties").strong());
                ui.separator();
                ui.add_space(4.0);
                if new_text_page.is_some() {
                    self.draw_new_text_properties(ui, page);
                } else if text_page.is_some() {
                    self.draw_run_properties(ui);
                } else {
                    self.draw_shape_properties(ui, page);
                }
            });
    }

    /// The size, colour, font and alignment of [`Self::new_text_box`] — the
    /// panel `begin_text_box` opens instead of [`Self::draw_run_properties`],
    /// since a box being composed has no existing run to seed a size or
    /// position from.
    fn draw_new_text_properties(&mut self, ui: &mut egui::Ui, page: usize) {
        if self.tab().new_text_box.is_none() {
            return;
        }
        self.draw_text_style(ui, true);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            // Not "Insert": the ribbon already has one of those, for a
            // page, and a test (and a reader) finding this button by its
            // label should not also find that one.
            if ui.button("Add to Page").clicked() {
                self.apply_new_text_box(ui);
            }
            if ui.button("Cancel").clicked() {
                self.tab_mut().new_text_box = None;
            }
        });

        if self.font_picker_open {
            self.draw_font_picker(ui, page);
        }
    }

    /// The Text Style block (see [`text_style_panel`]) for the run being edited
    /// (`new_box` false) or the new text box being composed (`true`) — one layout
    /// for both. **No position row: a page's words are moved by dragging them**
    /// (or the box's left handle), not by typing coordinates (reported from use).
    ///
    /// What does something: the font (the picker), size, colour, **bold and
    /// italic as the font family's own Bold/Italic face** — an error says so when
    /// the family has none installed — and, for a new box, left/centre/right.
    /// Everything else is drawn greyed with its reason on hover.
    fn draw_text_style(&mut self, ui: &mut egui::Ui, new_box: bool) {
        use text_style_panel as tsp;
        let black = pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 };
        let (face, size, color, align) = if new_box {
            let Some(b) = self.tab().new_text_box.as_ref() else { return };
            let align = match b.align {
                TextAlign::Left => tsp::Align::Left,
                TextAlign::Center => tsp::Align::Center,
                TextAlign::Right => tsp::Align::Right,
            };
            (b.face.clone(), b.size, b.color, Some(align))
        } else {
            let Some(e) = self.tab().editing_run.as_ref() else { return };
            // An explicit pick wins; short of that, the run's own current font
            // beats a flat "(automatic)" that never said which font that meant.
            (
                e.style.face.clone().or_else(|| e.current_face.clone()),
                e.style.size.unwrap_or(12.0),
                e.style.color.unwrap_or(black),
                None,
            )
        };
        let (bold, italic) = face.as_deref().map(tsp::style_of).unwrap_or((false, false));
        let look = tsp::Look {
            font: face.clone().unwrap_or_else(|| "(automatic)".into()),
            size,
            color: [color.r, color.g, color.b],
            bold,
            italic,
            underline: false,
            strike: false,
            script: tsp::Script::Normal,
            align,
        };
        let mut gates = tsp::Gates::all_off("Not built yet. It comes in the next stage.");
        gates.bold = None;
        gates.italic = None;
        if new_box {
            gates.left = None;
            gates.center = None;
            gates.right = None;
            gates.justify = Some("Justified text is not available for a new box yet.".into());
        } else {
            let why = || {
                Some("Alignment is for new text boxes for now. Drag the side handles of the box to set where the lines wrap.".to_string())
            };
            (gates.left, gates.center, gates.right, gates.justify) = (why(), why(), why(), why());
        }

        let changes = tsp::show(ui, egui::Id::new(("text-style", new_box)), &look, &gates);

        if changes.font_clicked {
            self.font_picker_open = !self.font_picker_open;
            if self.font_picker_open && self.system_fonts.is_none() {
                self.system_fonts = Some(system_fonts::list());
            }
        }
        if let Some(new_size) = changes.size {
            if new_box {
                if let Some(b) = self.tab_mut().new_text_box.as_mut() {
                    b.size = new_size;
                }
            } else if let Some(e) = self.tab_mut().editing_run.as_mut() {
                e.style.size = Some(new_size);
            }
        }
        if let Some([r, g, b]) = changes.color {
            let new = pdf_core::document::Color { r, g, b, a: color.a };
            if new_box {
                if let Some(held) = self.tab_mut().new_text_box.as_mut() {
                    held.color = new;
                }
            } else if let Some(e) = self.tab_mut().editing_run.as_mut() {
                e.style.color = Some(new);
            }
        }
        if changes.bold.is_some() || changes.italic.is_some() {
            self.pick_style_face(face.as_deref(), changes.bold.unwrap_or(bold), changes.italic.unwrap_or(italic));
        }
        if let (true, Some(a)) = (new_box, changes.align) {
            let picked = match a {
                tsp::Align::Left => Some(TextAlign::Left),
                tsp::Align::Center => Some(TextAlign::Center),
                tsp::Align::Right => Some(TextAlign::Right),
                tsp::Align::Justify => None,
            };
            if let (Some(picked), Some(b)) = (picked, self.tab_mut().new_text_box.as_mut()) {
                b.align = picked;
            }
        }
    }

    /// Switch the run or new box to the `bold`/`italic` face of `current`'s own
    /// family, from the installed and bundled fonts. Says so, and changes
    /// nothing, when the family has no such face.
    fn pick_style_face(&mut self, current: Option<&str>, bold: bool, italic: bool) {
        if self.system_fonts.is_none() {
            self.system_fonts = Some(system_fonts::list());
        }
        // A new box set in "(automatic)" is Helvetica, whose installed twin is Arial.
        let hint = current.unwrap_or("Arial").to_string();
        let bundled = self.writing_faces();
        let found = {
            let names: Vec<&str> = self
                .system_fonts
                .iter()
                .flatten()
                .map(|f| f.name.as_str())
                .chain(bundled.iter().map(String::as_str))
                .collect();
            text_style_panel::family_variant(&hint, bold, italic, &names)
        };
        match found {
            Some(name) => self.choose_face(Some(name)),
            None => self.say_error(format!(
                "there is no {} face of {} installed, so the font was left as it is.",
                match (bold, italic) {
                    (true, true) => "bold italic",
                    (true, false) => "bold",
                    (false, true) => "italic",
                    (false, false) => "regular",
                },
                text_style_panel::family_of(&hint)
            )),
        }
    }

    /// The Text Style of the one run [`Self::editing_run`] holds, and the Apply
    /// button, with why the engine refused when it did. See
    /// [`Self::draw_run_editor`] for the box those words are still typed
    /// into, which stays on the page.
    fn draw_run_properties(&mut self, ui: &mut egui::Ui) {
        let Some(edit) = self.tab().editing_run.as_ref() else { return };
        let refusal_said = edit.refusal.as_ref().map(|refusal| format!("Not applied: {}", refusal.reason));
        self.draw_text_style(ui, false);

        ui.add_space(8.0);
        if ui.button("Apply").clicked() {
            self.apply_editing_page();
        }
        // **Why the engine did not take it**, next to the button that asked: the
        // box stays open with what was typed, and the person is told what stood in
        // the way instead of finding the box gone and a red line in a history that
        // is shut. Gone with the box, or when a new try replaces it.
        if let Some(why) = refusal_said {
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(why).color(theme::danger())).wrap());
        }

        if self.font_picker_open {
            let page = self.tab_mut().editing_run.as_ref().map(|e| e.page).unwrap_or(self.tab_mut().page);
            self.draw_font_picker(ui, page);
        }
    }


    /// The run editor's font picker: every font Windows has installed,
    /// filtered as you type, chosen to write the run in.
    ///
    /// A plain list rather than an `egui::ComboBox` — a system can easily
    /// have several hundred fonts installed, and a combo box holding all of
    /// them open at once is not a picker, it is a wall of text. Populated
    /// once into `self.system_fonts` (see that field's own doc) and read
    /// from here on every frame the picker is open, not rescanned.
    fn draw_font_picker(&mut self, ui: &mut egui::Ui, page: usize) {
        let mut close = false;
        let mut chosen: Option<Option<String>> = None;

        egui::Window::new("Font")
            .id(egui::Id::new(("font-picker", page)))
            .collapsible(false)
            .resizable(false)
            .default_width(280.0)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("filter");
                    // **Focus, every frame this is open.** Without it,
                    // keyboard focus stayed wherever it was before the
                    // button that opened this was clicked — the run
                    // editor's own text box — so typing "to filter fonts"
                    // silently rewrote the words being edited instead, and
                    // whatever kept that box in view as its content changed
                    // scrolled the page along with it. Reported from use as
                    // "the screen moves upward while searching fonts".
                    let filter = ui.text_edit_singleline(&mut self.font_picker_filter);
                    if !filter.has_focus() {
                        filter.request_focus();
                    }
                    if ui.small_button("×").clicked() {
                        close = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    if ui.selectable_label(false, "(automatic)").clicked() {
                        chosen = Some(None);
                    }
                    let filter = self.font_picker_filter.to_ascii_lowercase();
                    for font in self.system_fonts.iter().flatten() {
                        if !filter.is_empty() && !font.name.to_ascii_lowercase().contains(&filter) {
                            continue;
                        }
                        if ui.selectable_label(false, &font.name).clicked() {
                            chosen = Some(Some(font.name.clone()));
                        }
                    }
                });
            });

        if let Some(face) = chosen {
            self.choose_face(face);
            close = true;
        }

        if close {
            self.font_picker_open = false;
            self.font_picker_filter.clear();
        }
    }

    /// Use `face` for the run or new text box being edited, registering the
    /// font file for both writing paths first. `None` is "automatic".
    fn choose_face(&mut self, face: Option<String>) {
        {
            // The file this picks a font by is read once, here, rather than
            // every font on the machine being loaded up front — see
            // `system_fonts::list`, which reads only enough of each file to
            // find its name.
            if let Some(name) = &face {
                if let Some(path) = self
                    .system_fonts
                    .iter()
                    .flatten()
                    .find(|f| &f.name == name)
                    .map(|f| f.path.clone())
                {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            // A new text box writes with `pdf_core::text::
                            // shape` directly (see `write_styled_line_at`),
                            // which reads from `pdf_core::text`'s own
                            // name-keyed registry — a different pool from
                            // `add_typing_font`'s, which is what a run's own
                            // restyle (`SetTextRun`) draws from instead.
                            // Registered for both so either path can find it
                            // by this exact name.
                            if !pdf_core::text::is_registered(name) {
                                let _ = pdf_core::text::register(name, bytes.clone());
                            }
                            if let Some(doc) = &self.tab_mut().doc {
                                let _ = doc.session.add_typing_font(bytes);
                            }
                        }
                        Err(e) => self.say_error(format!("could not read {}: {e}", path.display())),
                    }
                }
            }
            if let Some(edit) = self.tab_mut().editing_run.as_mut() {
                edit.style.face = face;
            } else if let Some(new_text) = &mut self.tab_mut().new_text_box {
                new_text.face = face;
            }
        }
    }

    /// An outline around whatever the layer rail has picked.
    ///
    /// **A list of labels is not enough to pick from.** "words: Project
    /// Category" names one of several things that could be under the pointer,
    /// and the only way to know which is to see it on the page.
    fn draw_picked_layer(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        if !self.show_layers {
            return;
        }
        let Some(at) = self.tab_mut().picked_layer else { return };
        let Some(entry) = self.layers_on(page).get(at).cloned() else { return };

        let outline = egui::Rect::from_min_max(
            view.to_screen(AppPoint::new(entry.rect.left as f64, entry.rect.top as f64)),
            view.to_screen(AppPoint::new(entry.rect.right as f64, entry.rect.bottom as f64)),
        );
        let painter = ui.painter();
        painter.rect_stroke(
            outline.expand(1.5),
            egui::CornerRadius::ZERO,
            egui::Stroke::new(2.0, theme::violet()),
            egui::StrokeKind::Outside,
        );
        painter.rect_filled(
            outline,
            egui::CornerRadius::ZERO,
            theme::violet().gamma_multiply(0.12),
        );
    }


    fn draw_text_highlights(&mut self, painter: &egui::Painter, page: usize, view: PageView) {
        // Only on the page it was made on, now that a selection outlives the
        // page being scrolled past.
        let selection =
            (page == self.tab_mut().selection_page).then(|| self.tab_mut().text_selection.clone()).flatten();
        let hits: Vec<std::ops::Range<usize>> = self.tab_mut()
            .find_hits
            .iter()
            .filter(|(p, _)| *p == page)
            .map(|(_, range)| range.clone())
            .collect();
        let find_at = self.tab().find_at;
        let current = self.tab().find_hits.get(find_at).cloned();

        // **Before asking the engine for the page's text, not after.** This
        // runs for every visible page on every frame, and extracting the
        // characters goes through the one lock a render in progress also
        // holds — so a page with nothing to draw was waiting on the render
        // worker for nothing, and with two pages on screen each one evicted
        // the other from the one-page cache.
        if hits.is_empty() && selection.is_none() {
            return;
        }
        let Some(chars) = self.characters(page) else { return };

        let wash = |range: std::ops::Range<usize>, colour: egui::Color32| {
            for rect in chars.line_rects(range) {
                let min = view.to_screen(AppPoint::new(rect.left as f64, rect.top as f64));
                let max = view.to_screen(AppPoint::new(rect.right as f64, rect.bottom as f64));
                painter.rect_filled(
                    egui::Rect::from_min_max(min, max),
                    egui::CornerRadius::ZERO,
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

}

fn view_height(app: &PagifyApp, page: usize) -> f64 {
    app.tab().doc
        .as_ref()
        .and_then(|d| d.strip.size_of(page))
        .map(|(_, h)| h as f64)
        .unwrap_or(792.0)
}

/// The middle of the box round `objects`, in page space — what a paste of
/// shapes is centred on the pointer by.
fn shapes_centre(objects: &[(cad_kernel::DObject, bool)], layer: &pagify_shell::markup::Layer) -> AppPoint {
    let (mut min, mut max) = (cad_kernel::Vec2::new(f64::MAX, f64::MAX), cad_kernel::Vec2::new(f64::MIN, f64::MIN));
    for (object, _) in objects {
        let (lo, hi) = object.bbox();
        min = cad_kernel::Vec2::new(min.x.min(lo.x), min.y.min(lo.y));
        max = cad_kernel::Vec2::new(max.x.max(hi.x), max.y.max(hi.y));
    }
    layer.space().from_kernel(cad_kernel::Vec2::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0))
}

/// Make the paper around a copied shape see-through: every near-white pixel
/// **reachable from the edge of the picture** without crossing ink becomes
/// transparent. White *inside* the shape (a white fill, a letter's counter) is
/// not reachable and stays, so the shape is not holed.
fn clear_paper_around(rgba: &mut [u8], width: usize, height: usize) {
    let is_paper = |px: &[u8]| px[3] > 0 && px[0] >= 250 && px[1] >= 250 && px[2] >= 250;
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return;
    }
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..width {
        stack.push(x);
        stack.push((height - 1) * width + x);
    }
    for y in 0..height {
        stack.push(y * width);
        stack.push(y * width + width - 1);
    }
    while let Some(i) = stack.pop() {
        let at = i * 4;
        if !is_paper(&rgba[at..at + 4]) {
            continue;
        }
        rgba[at + 3] = 0;
        let (x, y) = (i % width, i / width);
        if x > 0 {
            stack.push(i - 1);
        }
        if x + 1 < width {
            stack.push(i + 1);
        }
        if y > 0 {
            stack.push(i - width);
        }
        if y + 1 < height {
            stack.push(i + width);
        }
    }
}

/// Appends `piece` to `job` in the document's face (`doc`), except for the
/// characters `covered` says that face cannot draw, which go in `fallback`
/// — see [`editor_sections`]. `leading_space` goes before the first section
/// only.
///
/// The job's text grows by exactly `piece`, whatever the split: the cursor and
/// the selection are positions in the buffer, and a job whose text drifted
/// from it would put them in the wrong place. A piece with nothing to move is
/// appended in one go, exactly as it always was.
fn append_sectioned(
    job: &mut egui::text::LayoutJob,
    piece: &str,
    leading_space: f32,
    doc: &egui::TextFormat,
    fallback: &egui::TextFormat,
    covered: &dyn Fn(char) -> bool,
) {
    let sections = editor_sections(piece, covered);
    if sections.iter().all(|(_, drawable)| *drawable) {
        job.append(piece, leading_space, doc.clone());
        return;
    }
    let mut lead = leading_space;
    for (range, drawable) in sections {
        job.append(&piece[range], lead, if drawable { doc.clone() } else { fallback.clone() });
        lead = 0.0;
    }
}

/// The `LayoutJob` the run editor lays its buffer out with — one line at a
/// time, so each line of a justified paragraph can be stretched to the box's
/// width by hand. `measure` answers how wide a word is, in the document's
/// face; `covered` and `fallback` are [`append_sectioned`]'s.
///
/// **Justified, the same as the page it came from — real justification, not
/// left-aligned and ragged.** `egui::TextEdit` has no setting for this; the
/// job is built by hand instead, which is what lets it insert exactly the
/// extra space `justify_gaps` computes for a line rather than leaving every
/// gap at its natural width. See that function's own doc for why
/// `LayoutJob::justify` itself is not enough here.
///
/// The paragraph's own last line is excluded, matching ordinary typesetting —
/// a short closing line is not stretched to fill the column just because the
/// lines above it were.
fn editor_layout_job(
    text: &str,
    justify: bool,
    wrap_width: f32,
    doc: &egui::TextFormat,
    fallback: &egui::TextFormat,
    covered: &dyn Fn(char) -> bool,
    measure: &mut dyn FnMut(&str) -> f32,
) -> egui::text::LayoutJob {
    // No `job.wrap.max_width` here — every line break is already explicit (one
    // object per line, joined by `\n`, or a fresh one just typed), and a
    // justified line's own stretch is computed by hand below to land right at
    // `wrap_width`. Wrapping at that same width risked pushing a line's last
    // word onto a row of its own the moment the stretch rounded a pixel or two
    // long — and for an unjustified line, auto-wrap would silently turn one
    // typed line into two nobody asked for.
    let mut job = egui::text::LayoutJob::default();
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    for (li, line) in lines.iter().enumerate() {
        if li > 0 {
            job.append("\n", 0.0, doc.clone());
        }
        // `split_inclusive` keeps each word's own trailing space attached to
        // it — the actual space character stays in the job's text untouched
        // (cursor positions must match the buffer exactly); `leading_space`
        // only ever adds a further, purely visual nudge on top of it.
        let tokens: Vec<&str> = line.split_inclusive(' ').collect();
        if !justify || li == last || tokens.len() < 2 {
            append_sectioned(&mut job, line, 0.0, doc, fallback, covered);
            continue;
        }
        let widths: Vec<f32> = tokens.iter().map(|t| measure(t.trim_end())).collect();
        let gaps = justify_gaps(&widths, wrap_width);
        for (token, extra) in tokens.iter().zip(gaps) {
            append_sectioned(&mut job, token, extra, doc, fallback, covered);
        }
    }
    job
}








#[cfg(test)]
mod majority_look_tests;










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
    paste: bool,
    find_next: bool,
    open: bool,
    save: bool,
    undo: bool,
    redo: bool,
    search: bool,
    close_tab: bool,
    next_tab: bool,
    prev_tab: bool,
    print: bool,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod text_layer_tests;

#[cfg(test)]
mod unsaved_guard_tests;

#[cfg(test)]
mod reading_tests;

/// Check Spelling, the small items: the suggestions are worked out once per
/// word, the checker does not judge what its English list cannot judge, and a
/// Change the engine refuses says so.
///
/// Driven through the panel itself (`harness`, a click on its buttons), because
/// the first of these is a per-frame cost and cannot be seen from a method call.
#[cfg(test)]
mod g3_spell_tests;

#[cfg(test)]
mod spell_check_tests;

#[cfg(test)]
mod undo_wiring_tests;

/// The two things a user does with a mouse: draw with the tools, and select
/// text. Both were only ever exercised through the command box, where every
/// point arrives as typed coordinates — which is not how anybody uses them.
#[cfg(test)]
mod pointer_tests;

/// Driving the real interface, not the methods behind it.
///
/// Selection and the drawing tools broke twice while every logic test stayed
/// green, because the logic was never what broke: a mode swallowed the drag, a
/// panel took the pointer, a scroll area consumed the gesture. None of that is
/// reachable from a method call, and all of it is what the user actually
/// touches. These build the whole window and use a mouse on it.
#[cfg(test)]
mod ui_tests;

/// Find and the view: where a match lands on screen, what Replace does next,
/// what drawing a highlight costs, and what the File tab does to the page you
/// were reading.
///
/// **Measured off the real frame.** The match tests read the amber wash
/// `draw_text_highlights` actually painted for the current match out of the
/// frame's shapes and ask whether it is inside the window the page scrolls
/// in — not where the view ought to be by some model of the layout, which
/// would pass for any model that agreed with the fix.
#[cfg(test)]
mod g2_find_view_tests;

#[cfg(test)]
mod raster_scale_tests;

#[cfg(test)]
mod g4_edit_error_tests;

#[cfg(test)]
mod run_editor_font_size_tests;

#[cfg(test)]
mod run_editor_box_grow_tests;

#[cfg(test)]
mod run_editor_glyph_size_tests;

#[cfg(test)]
mod quantize_to_nearest_8_tests;

/// **Reported from use (report 10): Extract "lacks a UI, only the raw command
/// works"** — the three faults under it that are not the missing dialog.
/// Space ran the line before a second word could be typed; a prefilled line
/// took what was typed in front of it; and three buttons ran a bare command
/// that could only fail. Every test here types through the real command bar
/// with real key events: the `submit(&str)` shortcut the rest of the suite
/// uses never passes through any of this, which is why none of it was caught.
#[cfg(test)]
mod g1_command_box_tests;

/// The update script, run for real against real files — a Windows batch file,
/// so only on Windows. The stand-in for "another Pagify window" is a program
/// that is genuinely running under the name `Pagify.exe`, because what
/// Windows refuses to overwrite is a running program's file and nothing else
/// reproduces that.
/// **Asked for with a screenshot: reference lines while something is moved.**
/// Grey lines through the edges and middles of what is near, green where a side
/// or the middle lines up, the thing pulled onto it — and none of it unless a
/// thing is actually being moved.
#[cfg(test)]
mod guide_tests;

/// **Asked for with a screenshot: a rotate handle on a selected object, like the
/// move and resize handles, and the angle shown while it is turned.**
#[cfg(test)]
mod rotate_handle_tests;

/// **Reported from use, twice: "pdfium error: PdfiumLibraryInternalError(
/// Unknown) ... PDFium was looked for at ..." opening a file.** A file an older
/// build damaged when it saved it opens now, and says so; any other file that
/// will not open says what is wrong with *it*, not where PDFium was looked for.
#[cfg(test)]
mod open_error_tests;

/// **Reported from use: with many documents open the tabs stacked into five
/// rows and took the page's space.** One row, the newest on the left, and the
/// ones that do not fit behind a small triangle.
#[cfg(test)]
mod tab_strip_tests;

/// **Reported from use: links inside a PDF do nothing.** A contents entry or a
/// "back to the index" is a link to another page; only web addresses were ever
/// followed.
#[cfg(test)]
mod internal_link_tests;

#[cfg(all(test, windows))]
mod update_script_tests;

#[cfg(test)]
mod update_check_tests;

#[cfg(test)]
mod paragraph_should_justify_tests;

#[cfg(test)]
mod ribbon_overflow_tests;

#[cfg(test)]
mod justify_gaps_tests;

#[cfg(test)]
mod join_paragraph_lines_tests;

#[cfg(test)]
mod fix_extracted_text_tests;

/// A line-end hyphen the page's text carries as a control code, before a line
/// the page draws as shapes — the one place [`fix_extracted_text`] cannot see the
/// letter that follows, and used to drop the hyphen.
#[cfg(test)]
mod drawn_line_hyphen_tests;

#[cfg(test)]
mod looks_rotated_tests;

#[cfg(test)]
mod redaction_wiring_tests;

#[cfg(test)]
mod lock_wiring_tests;

/// Draw a signature inside a box, fitted and centred.
///
/// Uses [`Signature::placed`] — the same arithmetic that puts one on a page —
/// so a preview cannot flatter a signature that will land differently.
fn paint_signature(
    painter: &egui::Painter,
    area: egui::Rect,
    signature: &pagify_shell::signatures::Signature,
    texture: Option<&egui::TextureHandle>,
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

    if let Some(texture) = texture {
        let rect = egui::Rect::from_min_size(
            egui::pos2(left, baseline - height),
            egui::vec2(width, height),
        );
        painter.image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        return;
    }

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

/// The six-letter, all-caps subset tag a PDF generator writes onto a
/// font's own `/BaseFont` (`"DOHVNI+Montserrat-Light"`), stripped —
/// leaving the family name a second, differently-subset copy of the same
/// face shares (`"Montserrat-Light"`). The six letters exist only so two
/// subsets of one face never collide inside a single document's
/// `/Resources`; they say nothing about which glyphs either one has.
fn strip_subset_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.len() > 7 && bytes[6] == b'+' && bytes[..6].iter().all(u8::is_ascii_uppercase) {
        &name[7..]
    } else {
        name
    }
}

/// The paragraph editor's wrap hyphens, end to end on small hand-written
/// pages: what opening a paragraph puts in the buffer, and — the part that
/// matters — what applying it writes back into the page.
#[cfg(test)]
mod wrap_hyphen_tests;

/// The editor's blank glyphs: a letter the document's embedded subset maps but
/// never drew must come out in the program's own face, not as nothing.
#[cfg(test)]
mod editor_blank_glyph_tests;

/// What applying an edit does to a paragraph: **a retyped line replaces its
/// pieces** (the first takes the words, the others come off the page), a line
/// the edit does not change is left exactly as it was — every piece of it — a
/// frozen line is never touched, a paragraph that no longer lines up with its
/// own text is refused rather than guessed at, and the whole apply undoes, and
/// redoes, as one exact step.
#[cfg(test)]
mod paragraph_apply_tests;

#[cfg(test)]
mod pick_wiring_tests;
