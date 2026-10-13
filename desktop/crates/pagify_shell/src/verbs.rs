//! Pagify's own verbs, and the two policies that keep them honest next to
//! `cad_kernel`'s.
//!
//! ## The namespaces overlap, and the plan underestimates by how much
//!
//! Build plan §7 presents the split as clean: the kernel owns the drawing verbs
//! (`l`, `pl`, `tr`, `f`, `o`) and Pagify owns its own (`open`, `save`,
//! `rotate`, `extract`, `redact`, `sign`). Measured against the kernel's actual
//! table, that is not the shape of the problem. `cad_kernel::parser` already
//! claims **`open`, `save`, `saveas`, `rotate`, `undo`, `redo`, `help`** — most
//! of §7's own examples.
//!
//! "Try Pagify's table, fall through to the kernel's" therefore *shadows* real
//! kernel verbs, silently, by default. The rule here is that shadowing is
//! allowed but never accidental: every collision is listed in
//! [`DELIBERATE_OVERRIDES`] with the reason, and `tests/command_box.rs` fails
//! on any collision that is not. A SIMLUX release that adds a verb colliding
//! with one of ours breaks a test rather than quietly changing what a word
//! means for people who use both programs.
//!
//! ## Refusal is where CAD-only verbs are declined
//!
//! §6 says to "leave" `wall`, `wallstyle`, `blockdiff` and `units`. That cannot
//! be done by not importing them — `Wall` is a variant of `Geom` itself, and
//! the kernel's parser is a single function that claims all of them. So the
//! declining happens *here*, at dispatch, in [`REFUSED`]: one list, in one
//! place, of what Pagify is not. The kernel stays forkless (§5.4) and §9's
//! "scope drift back into CAD" has a single site to review.

use std::path::PathBuf;

use pdf_core::document::Ruling;

/// What a reader may do with a secured document.
///
/// **A request, not a guarantee** — and the difference matters enough to say
/// out loud. What actually withholds a document is the password; these flags
/// are honoured by readers that choose to, and the PDF specification says as
/// much. Withholding printing from someone who can already read the page is a
/// courtesy, not a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecureOptions {
    pub printing: bool,
    pub copying: bool,
    pub editing: bool,
    pub annotating: bool,
}

impl Default for SecureOptions {
    /// Everything permitted, which is what "put a password on this" means when
    /// nothing else is said.
    fn default() -> Self {
        SecureOptions { printing: true, copying: true, editing: true, annotating: true }
    }
}

impl SecureOptions {
    /// Read the words after `secure`.
    ///
    /// `secure` alone permits everything. `readonly` forbids the lot, and the
    /// `no…` words forbid one thing each — combinable, and readable aloud:
    /// `secure noprint nocopy`.
    ///
    /// An unknown word is refused rather than ignored. Silently permitting
    /// something a person had just tried to forbid is the worst outcome
    /// available here.
    pub fn parse(tail: &str) -> Result<SecureOptions, String> {
        let mut options = SecureOptions::default();
        for word in tail.split_whitespace() {
            match word.to_ascii_lowercase().as_str() {
                "readonly" | "read-only" => {
                    options = SecureOptions {
                        printing: false,
                        copying: false,
                        editing: false,
                        annotating: false,
                    }
                }
                "noprint" | "noprinting" => options.printing = false,
                "nocopy" | "nocopying" => options.copying = false,
                "noedit" | "noediting" => options.editing = false,
                "noannotate" | "noannotating" | "nocomment" => options.annotating = false,
                other => {
                    return Err(format!(
                        "secure: don't know {other:?} — try `readonly`, or any of \
                         `noprint` `nocopy` `noedit` `noannotate`"
                    ))
                }
            }
        }
        Ok(options)
    }

    /// What was allowed, in words, for telling somebody what they just did.
    pub fn describe(&self) -> String {
        let forbidden: Vec<&str> = [
            (!self.printing).then_some("printing"),
            (!self.copying).then_some("copying"),
            (!self.editing).then_some("editing"),
            (!self.annotating).then_some("annotating"),
        ]
        .into_iter()
        .flatten()
        .collect();

        match forbidden.len() {
            0 => "everything permitted".into(),
            4 => "reading only".into(),
            _ => format!("no {}", forbidden.join(", no ")),
        }
    }
}

/// What the Manage Signatures tool has been asked to do.
///
/// Renaming takes only the new name and acts on the current signature: a
/// command taking two names could not tell where the first ended, and
/// "Signature 2" is exactly the sort of name people give these.
#[derive(Debug, Clone, PartialEq)]
pub enum Signatures {
    /// Show the list, with each signature drawn.
    Open,
    /// Say what is kept, in the command box.
    List,
    /// Make one the one a click places.
    Use(String),
    /// Forget one. Not undoable.
    Forget(String),
    /// Rename the current one.
    Rename(String),
}

/// What `signature` was asked to do — see [`Verb::Signature`].
#[derive(Debug, Clone, PartialEq)]
pub enum SignatureAction {
    /// Place the last signature made, drawn or uploaded.
    Place,
    /// Draw a new one by hand.
    Draw,
    /// Add one from a picture file. `None` asks for a native file picker;
    /// `Some(path)` is one already chosen or typed.
    Upload(Option<PathBuf>),
}

/// What `addshape` and `drawing` answer with — the shape words this build
/// actually has. Kept here so the answer and the help text cannot drift.
pub const SHAPE_WORDS: &str = "shapes draw with their own words: `line`, `rectangle`, `circle`, \
                                `arrow`, `pline` and `spline` — and freehand ink is `pencil`.";

/// What `readingoptions` answers with — the reading options this build
/// actually has, and the one it does not.
pub const READING_OPTIONS_WORDS: &str = "reading options in this build live in `viewsetting` — \
                                          page display, turn and zoom — with `changecolor` for \
                                          the paper. Speech, the Read Out Loud side of reading \
                                          options, is not built.";

/// What `keeptool` answers with: the state of the "keep tool selected"
/// question in this build, which is that the keeping is already the default.
pub const KEEP_TOOL_WORDS: &str = "every comment tool stays in hand already — a run of ticks, \
                                   notes or strokes runs until Escape. The only tools that put \
                                   themselves down on purpose are the signature, a placed \
                                   picture, typed text and calibration, each because what \
                                   follows wants the pointer back.";

/// The colour behind the page — the View tab's Change Color.
///
/// Only the choice, no colours: the shell has no window, and the palette is
/// the app's (`paper_tint_color`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaperTint {
    /// What the background has always been — the theme's own.
    #[default]
    White,
    Grey,
    Sepia,
    Dark,
}

impl PaperTint {
    /// The next one in the ring a bare `changecolor` steps through:
    /// white → grey → sepia → dark → white.
    pub fn next(self) -> PaperTint {
        match self {
            PaperTint::White => PaperTint::Grey,
            PaperTint::Grey => PaperTint::Sepia,
            PaperTint::Sepia => PaperTint::Dark,
            PaperTint::Dark => PaperTint::White,
        }
    }

    /// The word the command bar and the panel use for it.
    pub fn word(self) -> &'static str {
        match self {
            PaperTint::White => "white",
            PaperTint::Grey => "grey",
            PaperTint::Sepia => "sepia",
            PaperTint::Dark => "dark",
        }
    }
}

/// A Pagify verb: something that acts on the document rather than on geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum Verb {
    Open(PathBuf),
    /// Ask for a file, rather than being told one. `open` with no path.
    OpenDialog,
    /// `force` is the `!` form — `close!` — which discards unsaved marks.
    ///
    /// Spelt as a separate word rather than answered at a prompt so that it is
    /// typeable, scriptable and recordable like everything else. A modal dialog
    /// would be a fourth kind of thing the Automate tab could not replay.
    Close { force: bool },
    Save,
    SaveAs(PathBuf),
    /// Ask for a destination, rather than being told one. `saveas` with no
    /// path — see [`Verb::OpenDialog`], which is the same idea for opening.
    SaveAsDialog,
    Page(PageTarget),
    Zoom(ZoomTarget),
    /// Rotate the *page*, in degrees clockwise. Quarter turns only — PDF stores
    /// page rotation as one of four values, not as an arbitrary angle.
    RotatePage(i32),
    Undo,
    Redo,
    /// Arm the lock tool: drag a rectangle, and everything inside it comes off
    /// the page and into the document, sealed under a passcode.
    ///
    /// **Lock hides; Redact destroys.** Two verbs rather than a flag on one,
    /// because the difference is the whole point and a flag is the kind of thing
    /// people get wrong once.
    Lock,
    /// Arm the rectangle form of the lock tool — `lockarea`.
    ///
    /// Kept because a selection cannot say everything: an area of a scan has no
    /// text to select, and a region of a drawing is not words at all. But it is
    /// no longer what `lock` does on its own, because being asked for two
    /// opposite corners is not what anyone reaches for when they mean "hide
    /// these words".
    LockArea,
    /// Lock whole pages rather than an area of one — `lock all`, `lock 1-3`.
    ///
    /// A separate variant rather than an `Option<String>` on [`Verb::Lock`],
    /// because the two do genuinely different things: one arms a tool and waits
    /// for a drag, the other acts at once on pages already named. They also
    /// succeed on different documents — whole pages need no content identified,
    /// so this works where an area lock refuses.
    LockPages(String),
    /// Show what a page draws, in the order it draws it.
    ///
    /// **A PDF has no z-index.** What is drawn later covers what came before,
    /// so the page's own drawing order *is* its stacking, and a list of it in
    /// that order is the only honest layer panel there is.
    Layers,
    /// Put the thing that is picked at the front of the page's drawing order.
    BringToFront,
    /// Put it at the back.
    SendToBack,
    /// How see-through the selected thing is drawn, as a percentage.
    Opacity(f32),
    /// Finish any lock whose badge stands over a picture still on the page.
    ///
    /// Done on every open without being asked; this is for a document already
    /// open when the badge was noticed.
    RepairLocks,
    /// Bring back everything a passcode has sealed in this document.
    Unlock,
    /// Put a password on the file — one any reader will ask for.
    ///
    /// **A different promise from [`Verb::Lock`].** Lock takes content off a
    /// page and keeps a sealed copy inside the file: the words are gone for
    /// anyone without the passcode, but the document still opens for everybody.
    /// This encrypts the whole file, so nothing at all is readable without the
    /// password — in Pagify or anywhere else.
    Secure(SecureOptions),
    /// Take the password off again, before it has been saved with one.
    Unsecure,
    /// Write something kept for writing again.
    ///
    /// A name, an address, a reference number — what filling a form in means
    /// typing over and over. With words, it keeps them and writes them; with
    /// none, it shows what is kept.
    ///
    /// **Only what is handed to it is kept.** Nothing written with `addtext`
    /// reaches the list: a form field holds somebody's name and account number,
    /// and copying all of that to disk because it might be handy later is not a
    /// decision this program makes for them.
    PredefinedText(Option<String>),
    /// Move something that is not words — a picture, a drawn shape.
    ///
    /// The same two clicks as [`Verb::MoveThing`], and it will move words too
    /// where there is nothing else under the pointer. What differs is which of
    /// two overlapping things is meant: here, the picture.
    EditObject,
    /// Pick something up off the page and put it down somewhere else.
    ///
    /// Words, a picture — whatever is under the first click. Two clicks: what
    /// to move, and where it goes.
    MoveThing,
    /// Rule a line while filling a form in.
    ///
    /// A fill-and-sign mark, in the same ink as the tick and the box. **Not**
    /// the `line` drawing tool, which makes a mark laid over the page that can
    /// be selected, moved and deleted.
    SignLine,
    /// Draw a box around something while filling a form in.
    ///
    /// A fill-and-sign mark, in the same ink as the tick and the cross, written
    /// into the page. **Not** the `rectangle` drawing tool, which makes a mark
    /// laid over the page that can be selected, moved and deleted.
    SignRectangle,
    /// Say what protects this document, and what does not.
    ///
    /// One readout rather than five tools asked in turn: the password, what it
    /// permits, the signatures and whether they still hold, the marks placed
    /// but not applied, what is locked, and how it must be saved.
    DocumentStatus,
    /// Where this run's session log is being written — see
    /// [`crate::session_log`]. A running transcript of every command and
    /// every line the app has said about it, kept so a bug can be reproduced
    /// once and the file handed over, rather than described from memory.
    SessionLog,
    /// Burn every placed signature into the page it sits on.
    ///
    /// After this they are page content rather than annotations: nobody can
    /// select one and delete it, and nothing here can lift one back out. The
    /// escape is the one that has always been there — close without saving.
    ApplySignatures,
    /// Look after the signatures you have drawn.
    ///
    /// The *drawings*, not the marks already on a page — a signature placed in
    /// a document is part of that document, and comes off with undo or with
    /// the annotation tools. This is the list somebody draws from.
    ManageSignatures(Signatures),
    /// Place a signature you have drawn.
    ///
    /// **Ink, not a certificate.** This is the shape of a name, stamped where
    /// somebody clicks — the mark a person writes on a form, drawn with a
    /// mouse or uploaded from a picture. It says nothing about who made it,
    /// and the tool that does is `certify`. Anything that blurs the two is
    /// worse than no signature at all.
    Signature(SignatureAction),
    /// Check the signatures this document carries.
    ///
    /// Says whether the file has changed since it was signed. **Not** whether
    /// the signer is who they say — that needs a chain of trust this program
    /// does not have, and every answer says so.
    Validate,
    /// Sign the document with a certificate — or report what signatures it has.
    ///
    /// `certify` says; `certify <file.p12>` signs with that identity. **A real
    /// signature**, over the bytes of the file, which any reader can check —
    /// and which anything written afterwards would break.
    Certify(Option<PathBuf>),
    /// Fill a form in by hand: type where you click, or put a tick, a cross or
    /// a dot there.
    ///
    /// **For documents that have no form fields**, which is most of the ones
    /// people are sent. Nothing here is an annotation: it goes into the page's
    /// own content, so it prints and cannot be switched off.
    FillSign(Option<String>),
    /// Mark how far a document may travel — or report how it is marked.
    ///
    /// `sensitivity` says; `sensitivity confidential` marks; `sensitivity none`
    /// takes the marking off. **A marking is not a control**: it stops nobody,
    /// and whoever offers it must say so rather than let it be mistaken for
    /// `secure` or the lock.
    Sensitivity(Option<String>),
    /// Paint over an area, covering what is there without removing it.
    ///
    /// **Not redaction, and never described as though it were.** The words
    /// underneath stay in the file and stay findable; this covers a blemish on
    /// a scan or a note in a margin. A separate verb rather than a flag on
    /// `redact`, because a flag that turns "destroy" into "cover" is exactly
    /// the kind of thing that gets set by accident.
    Whiteout,
    /// Two corners of an area to wash with the highlighter — the Comment
    /// tab's Area Highlight. Unlike the selection marks, it is drawn over
    /// anything, words or not.
    AreaHighlight,
    /// Freehand ink — the Comment tab's Pencil. Drag to draw a stroke; a
    /// click dots.
    Pencil,
    /// Whether the Rectangle and Circle drawing tools fill the shape solid or
    /// leave it hollow — chosen before drawing, not after.
    ///
    /// A toggle rather than a two-argument command (`fill on` / `fill off`):
    /// the ribbon button that runs this has one job, to flip whatever the
    /// state already is, and a caller who wants a specific state can just
    /// check the reply.
    ToggleFill,
    /// Arm the Draw tab's Arrow tool — a line with an arrowhead pre-picked
    /// on its end. Not a kernel word: `cad_kernel`'s own parser has no
    /// concept of an arrow, only of a line, so this is Pagify's own rather
    /// than a `Command::SetTool(ToolKind::_)` the kernel already recognises
    /// the way `line`/`circle`/`pline` are.
    DrawArrow,
    /// Find things on the pages somebody would not want to send out.
    ///
    /// `redact` is the form that acts on them. Two words rather than one for
    /// the same reason `hiddendata` has two: what this finds are **candidates**,
    /// and blacking them out unread is how a price somebody meant to send gets
    /// hidden and a name nobody recognised does not.
    SmartRedact { redact: bool },
    /// Report what a document carries that is not on its pages.
    ///
    /// `clean` is the form that removes it. Two words rather than one because
    /// a person deciding whether to sanitise needs to know what sanitising
    /// would cost — an author's name and a form's values are both hidden data,
    /// and only one of them is usually unwanted.
    HiddenData { clean: bool },
    /// Arm the redaction tool: drag a rectangle, and everything inside it is
    /// destroyed.
    ///
    /// **Not a mark drawn on top.** The text objects are deleted and the file is
    /// rewritten without them, which is why this cannot be an annotation and why
    /// saving afterwards has to rewrite the whole file.
    Redact,
    /// Supply the next click to a command that is waiting for one.
    ///
    /// §7's claim is that anything clickable is typeable. Without this, every
    /// interactive tool — fillet, trim, offset, measure — is reachable only
    /// with a pointer, which makes them unscriptable, unrecordable by the
    /// Automate tab, and untestable without a window.
    Pick(AppPointArg),

    // Organize — phase 10.
    /// Pull pages out into a new document.
    Extract { pages: String, dest: PathBuf },
    /// Bring pages in from another PDF, before the current one.
    Import { source: PathBuf, pages: String },
    DeletePages(String),
    /// A blank sheet the size of the current page, before it.
    InsertPage,
    /// Drag, as a command: move these pages in front of that one (one-based).
    MovePages { pages: String, before: usize },
    /// Replace the page in view with pages from another file —
    /// `replacepage <file> [pages]`. Bare, the app asks for the file and uses
    /// its first page.
    ReplacePage { source: Option<PathBuf>, pages: String },
    /// Alternate another file's pages into this one, starting after the page
    /// in view. Bare, the app asks for the file.
    Interleave { source: Option<PathBuf>, pages: String },
    /// Write every page as a PNG into a folder — `toimage <folder>`. Bare,
    /// the app asks for the folder.
    ToImage { dest: Option<PathBuf> },
    /// Write the document's text as one HTML file — `tohtml <file.html>`.
    /// Bare, the app asks for the file.
    ToHtml { dest: Option<PathBuf> },
    /// Write the document out as several files, `every` pages each —
    /// `split [<every>] <folder>`. Bare, the app asks for the folder and
    /// writes one page per file.
    Split { every: usize, dest: Option<PathBuf> },
    /// Count words and characters over the pages named, or the whole
    /// document — `wordcount [pages]`.
    WordCount { pages: String },
    /// Write the same words on every page — `pagemarks [topleft] <words>`,
    /// with `{n}` standing for the page number. Bottom-left unless `topleft`
    /// is said.
    PageMarks { top: bool, text: String },
    /// Make a document out of the clipboard's text — the Convert tab's From
    /// Clipboard. The platform hands the text back as a paste event.
    FromClipboard,
    /// Open a copy of a file as untitled, so the original is untouched —
    /// `fromtemplate <file.pdf>`. Bare, the app asks for the file.
    FromTemplate { source: Option<PathBuf> },
    /// Bake every page's annotations and form fields into its content —
    /// the Page tab's Flatten. Not undoable.
    Flatten,
    /// Embed a file in the document — `attach [file]`. Bare, the app asks
    /// for the file.
    Attach { source: Option<PathBuf> },
    /// Attach a file to the page in view as an annotation — `attachcomment
    /// [file]`. Bare, the app asks for the file.
    AttachComment { source: Option<PathBuf> },
    /// Open the Tags panel — the Accessibility tab's Tags Panel. It lists
    /// the page's structure tree; read-only in this build.
    TagSpanel,
    /// The alternate-text reading over pages named, or the whole document —
    /// the Accessibility tab's Set Alternate Text, which in this build
    /// reads rather than edits.
    AltText { pages: String },
    /// The reading-order reading: each page's tags in order, or word that it
    /// has none — the Accessibility tab's Reading Order, which in this
    /// build reads rather than edits.
    ReadingOrder { pages: String },
    /// The View tab's Split: a second, read-only pane over the same strip,
    /// with its own scroll.
    ViewSplit,
    /// Name where this build's reading options live — the Accessibility
    /// tab's Reading Options, answered with the routes that exist.
    ReadingOptions,
    /// The View tab's Reverse View: read each facing row right to left.
    ReverseView,
    /// Stamp words in a red ring at a click — `stamp <words>`. Bare, it is
    /// refused with the shape that works.
    Stamp { words: String },
    /// Words in a box with a leader to the point they describe —
    /// `callout <words>`.
    Callout { words: String },
    /// Start marking areas for redaction, or destroy what is marked — the
    /// Protection tab's Mark for Redaction. Two presses: nothing goes until
    /// the second.
    MarkRedact,
    /// Write every form field's name, kind and value as JSON —
    /// `formexport [file.json]`. Bare, the app asks for the file.
    FormExport { dest: Option<PathBuf> },
    /// Read that JSON back and apply what this build can set —
    /// `formimport [file.json]`. Bare, the app asks for the file.
    FormImport { dest: Option<PathBuf> },
    /// Write every field as CSV — `formtosheet [file.csv]`. Bare, the app
    /// asks for the file.
    FormToSheet { dest: Option<PathBuf> },
    /// Set a field's tooltip — `tooltip <field> <words>`.
    ToolTip { field: String, text: String },
    /// Clear every field this build can set — the Form tab's Reset Form.
    /// A clear, not a reset: the form's stored defaults are not readable in
    /// this build, and the answer says so.
    ResetForm,
    /// Name the shape words this build actually has — what the Edit tab's
    /// Add Shapes and the Comment tab's Drawing buttons answer with.
    ShapeWords,
    /// Answer the keep-tool-selected question as this build stands: the
    /// keeping is already the default.
    KeepTool,
    /// Start a new document out of nothing — `blankdoc [size] [ruling]`,
    /// sizes `a4`/`a3`/`letter`/`legal` (letter when nothing is said) and
    /// rulings `lined`/`grid`/`dots` (plain paper when nothing is said), in
    /// either order.
    BlankDoc { width_pt: f32, height_pt: f32, ruling: Ruling },
    /// Merge PDFs into one new document — `combine` or `combine <folder>`,
    /// which takes every PDF in the folder in name order. Bare, the app
    /// opens a file picker.
    Combine { folder: Option<PathBuf> },

    // Review — phase 10.
    /// Find text across the document.
    Find(String),
    /// Step to the next or previous match.
    FindStep { forward: bool },
    /// Open the Search & Replace panel.
    Replace,
    /// Open the Check Spelling panel.
    Spelling,
    /// Add a bookmark for the current page.
    Bookmark,
    /// Arm the Article Box tool, or (with a selection already made) mark it.
    ArticleBox,
    /// Arm the Web Links tool, or (with a selection already made) ask for
    /// the address to link it to.
    Weblinks,
    /// Join a text selection spanning more than one line or block into one
    /// editable paragraph, or (with nothing selected) say how to.
    JoinText,
    /// Arm Match Properties: the next text selection becomes the sample to
    /// copy from, and every one after that is retyped to match it.
    MatchProperties,
    /// Copy the current text selection.
    Copy,
    /// Rebuild this page's reading order from its geometry.
    ///
    /// A request, never a decision. Measured across real documents, a CAD
    /// drawing and a product catalogue both score as more disordered than a
    /// deliberately scrambled page — because positioned layout and paint order
    /// look identical to the measure. Only someone looking at the page can tell
    /// them apart.
    Reflow,
    /// A note anchored where the pointer last was.
    Note(String),

    // Measurement — phase 9.
    /// Arm the two-point calibration pick.
    Calibrate { distance: f64, unit: String },
    /// Report the calibration currently in force.
    Scale,
    Measure(MeasureKind),

    // Automate — phase 11.
    Record(String),
    StopRecording,
    Replay(PathBuf),

    /// List every annotation on this page, including ones made elsewhere.
    ListMarks,
    /// Remove an annotation by the number `marks` gave it.
    RemoveMark(usize),

    /// Mark the selected text: highlight, underline, strikeout, squiggly.
    MarkText(Markup),

    /// Change words already on the page. Click a run, then retype it.
    EditText,

    /// Write words onto the page. Click where they go, then type them.
    AddText(String),

    /// Place a picture onto the page. Click where it goes.
    ///
    /// `None` opens a file dialog; `Some(path)` is the scripted form, exactly
    /// as `signature upload` takes one — a picture cannot be typed the way
    /// words can, so a button press and a written command both have to name a
    /// file rather than carry the content itself.
    AddImage(Option<PathBuf>),

    /// How pages are arranged: one per row, or two as a spread.
    SetLayout(crate::reader::Layout),

    /// Change the sheet size, scaling the content to match.
    ResizePages { pages: String, width_pt: f32, height_pt: f32 },

    /// Trim what pages show, by an inset in points from each edge.
    CropPages { pages: String, margin: f32 },

    /// Copy pages, placing the copies straight after the originals.
    DuplicatePages(String),

    /// Turn the document back to front.
    ReversePages,

    /// Show or hide the Organize grid — a thumbnail view with multi-select,
    /// drag-to-reorder, and copy/paste across tabs, replacing the plain
    /// scrolling thumbnail rail for as long as it's open.
    Thumbnails,
    /// Flip between the dark and light palettes — see `theme::toggle`. The
    /// choice is remembered across launches.
    ToggleAppearance,
    /// Exchange two pages, one-based as typed.
    SwapPages { a: usize, b: usize },
    /// Turn pages a quarter-turn. Positive is clockwise.
    RotatePages { pages: String, quarters: i32 },

    /// Put the selected text on the clipboard.
    ///
    /// ⌘C already did this. The word did not exist, which meant the one thing
    /// a reader most wants to do with a selection could not be done from the
    /// command box, scripted, or recorded — and §7 says the box can do
    /// anything the interface can.
    CopyText,

    /// Put back whatever was last copied — a page, a drawn object, or
    /// nothing yet.
    ///
    /// **Reported from use**: `copy` on a selected page says "1 page copied
    /// — `paste` puts it in." — its own message names this exact word — and
    /// typing it answered "unknown command 'paste'.", because only ⌘V had
    /// ever been wired up. The one word the app tells you to type did not
    /// exist.
    Paste,

    /// Read the page with OCR and write what it finds back as an invisible
    /// text layer, so it can be selected, searched and copied.
    ///
    /// For the two kinds of page that look like text and are not: a scan, and
    /// a page whose words were drawn as glyph outlines. Both render perfectly
    /// and neither has a single character in it to select.
    ExtractText(String),

    /// Manage the extra fonts tried, alongside the bundled ones, when a page
    /// turns out to be outlined type rather than real text.
    OutlinedFont(OutlinedFontAction),
    /// Forget the recent-documents list, on disk too, and say where Pagify
    /// keeps what is its own.
    ClearHistory,

    /// What the pointer does on the page.
    ///
    /// The two standing tools at the head of every ribbon tab. They were the
    /// only buttons a user reaches for before anything else and the only two
    /// that answered "planned" — which reads as "this program cannot select
    /// text", because selecting text is what the Select tool is for.
    Pointer(PointerMode),

    /// Close a pick that has no fixed number of points — a polyline, an area
    /// measurement.
    ///
    /// The pointer finishes these with Enter. Without a word for it the command
    /// box cannot, which breaks §7's rule that the box is the single way
    /// anything happens: a recorded session could start a polyline and never
    /// end one.
    Finish,

    Help(Option<String>),
    /// Report which PDFium this build loaded. A wrong one presents as "some
    /// pages render oddly", so it is worth being answerable from the box.
    Pdfium,
    /// Say which build this is — otherwise unanswerable from the box, which
    /// makes "did the update actually take" unanswerable too.
    Version,
    /// Look again for a newer build without waiting for the next launch —
    /// mainly for confirming a publish landed, without relaunching.
    CheckUpdate,
    /// Say what this program is, in a line or two — the About button's word.
    About,
    /// The window's key bindings, listed in the box. The bindings themselves
    /// live in the app (`Keys` in `main.rs`); this only asks for them.
    Shortcuts,
    /// The first few things to try, for someone who just opened it.
    QuickStart,
    /// Say where a bug report goes, and where this run's session log is —
    /// the two things a report needs.
    ReportIssue,
    /// Say what kind of text this page has, and what that means.
    TextLayer,
    /// Put the page in view on the system clipboard as an image — the
    /// Capture tab's Snapshot.
    Snapshot,
    /// Scroll the view by itself, one frame at a time — the View tab's
    /// AutoScroll. `autoscroll` again, or Escape, stops it.
    AutoScroll,
    /// A ruler along the page in view's top and left edges, in page points —
    /// the View tab's Toggle Ruler.
    Ruler,
    /// Run the accessibility checks this build can make — the Accessibility
    /// tab's Full Check.
    AccessCheck,
    /// Write those findings as one HTML file — `accessreport [file.html]`.
    /// Bare, the app asks for the file.
    AccessReport { dest: Option<PathBuf> },
    /// Make a bookmark per page, named from the page's own largest text run
    /// — the Navigation tab's Auto Create Bookmarks.
    AutoBookmarks,
    /// Run the document checks this build can make — fonts embedded, image
    /// resolution — the Conversion tab's Preflight.
    Preflight,
    /// Write every embedded picture out as a PNG — `exportimages <folder>`.
    /// Bare, the app asks for the folder.
    ExportAllImages { dest: Option<PathBuf> },
    /// Open or close the View Settings panel — the View tab's View Setting.
    ViewSetting,
    /// Open or close the Accounting Calculator — the Comment tab's little
    /// adding machine.
    Calculator,
    /// Open or close the Comments panel — the Comment tab's Manage Comments:
    /// every annotation in the document, jump to it or take it off.
    ManageComments,
    /// Highlight every occurrence of the words — the Comment tab's Search &
    /// Highlight. `searchhighlight <words>`.
    SearchHighlight { words: String },
    /// Compare this document against another, page by page, in words — the
    /// View tab's Compare. `compare <file.pdf>`; bare, the app asks for the
    /// file.
    Compare { source: Option<PathBuf> },
    /// The colour behind the pages — the View tab's Change Color.
    /// `changecolor [white|grey|sepia|dark]`; bare, it steps to the next
    /// one.
    ChangeColor(Option<PaperTint>),
    /// Make a new document out of picture files — `fromfiles` or
    /// `fromfiles <folder>`, which takes every picture in the folder in name
    /// order. Bare, the app opens a picker.
    FromFiles { folder: Option<PathBuf> },
    Quit { force: bool },
    /// Recognised, deliberately not implemented yet, and carrying the phase
    /// that will implement it.
    ///
    /// Worth a variant of its own: a user who types `redact` should be told
    /// that the word is right and the feature is not here yet, which is a
    /// different fact from `unknown command 'redact'` — and it keeps the
    /// roadmap legible from inside the program.
    Planned { verb: &'static str, phase: &'static str },
}

/// What to do with the extra outlined-text fonts a reader can add beyond the
/// ones this build bundles — see `pagify_shell::outlined_fonts`.
#[derive(Debug, Clone, PartialEq)]
pub enum OutlinedFontAction {
    /// No path given: ask for one, the same way `open` with nothing typed
    /// does.
    Dialog,
    Add(PathBuf),
    Remove(PathBuf),
    Clear,
}

/// The four ways of marking words that a PDF calls *text markup*.
///
/// One PDF construct — a list of quadrilaterals over the words covered —
/// differing only in which mark a reader draws for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
}

impl Markup {
    pub fn name(self) -> &'static str {
        match self {
            Markup::Highlight => "highlighted",
            Markup::Underline => "underlined",
            Markup::StrikeOut => "struck out",
            Markup::Squiggly => "marked",
        }
    }
}

/// A paper size by name, or `WxH` in points.
///
/// The names are portrait; a landscape sheet is the numbers the other way
/// round, which is clearer typed than named.
pub fn paper_size(spec: &str) -> Option<(f32, f32)> {
    let name = spec.trim().to_ascii_lowercase();
    let named = match name.as_str() {
        "a3" => Some((841.89, 1190.55)),
        "a4" => Some((595.28, 841.89)),
        "a5" => Some((419.53, 595.28)),
        "letter" => Some((612.0, 792.0)),
        "legal" => Some((612.0, 1008.0)),
        "tabloid" => Some((792.0, 1224.0)),
        _ => None,
    };
    if named.is_some() {
        return named;
    }

    let (w, h) = name.split_once('x')?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    (w >= 1.0 && h >= 1.0).then_some((w, h))
}

/// What a drag on the page means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerMode {
    /// Drag selects: text where the drag starts on a character, marks where it
    /// starts on empty paper. The rule every PDF reader already teaches.
    #[default]
    Select,
    /// Drag scrolls the page.
    Pan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTarget {
    /// One-based, as typed. Converted to a zero-based index at the edge.
    Number(usize),
    Next,
    Previous,
    First,
    Last,
}

/// A point typed as `x,y` in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppPointArg {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureKind {
    Distance,
    Area,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomTarget {
    Factor(f32),
    In,
    Out,
    Fit,
    Width,
    Actual,
}

/// A Pagify verb that shadows a `cad_kernel` one, and why that is correct.
pub struct Override {
    pub token: &'static str,
    /// What the kernel would otherwise do with this word.
    pub kernel_meaning: &'static str,
    pub why: &'static str,
    /// The kernel alias that still reaches the shadowed behaviour, if any.
    pub kernel_keeps: Option<&'static str>,
}

pub const DELIBERATE_OVERRIDES: &[Override] = &[
    Override {
        token: "open",
        kernel_meaning: "open a .dxf / .rsm drawing",
        why: "Pagify opens PDFs and files related to them — never drawings. The \
              kernel's `open` takes a .dxf or .rsm, which is not a format this \
              program handles at all, and §10 rules out a drawing document \
              entirely. Falling through would answer `open report.pdf` by \
              trying to parse a PDF as a drawing.",
        kernel_keeps: None,
    },
    Override {
        token: "save",
        kernel_meaning: "write a .dxf / .rsm drawing",
        why: "The PDF is the document, and the only thing this program writes. \
              Saving must go through pdf_core's incremental save so a signature \
              survives the edit — never the kernel's .dxf/.rsm writer.",
        kernel_keeps: None,
    },
    Override {
        token: "saveas",
        kernel_meaning: "write a .dxf / .rsm drawing to a path",
        why: "Same as `save`.",
        kernel_keeps: None,
    },
    Override {
        token: "rotate",
        kernel_meaning: "rotate the current selection",
        why: "On a page, the unqualified word means the page — that is the \
              common operation and the one a reader reaches for. Selection \
              rotate is a phase 6 modify tool and keeps SIMLUX's own alias.",
        kernel_keeps: Some("ro"),
    },
    Override {
        token: "undo",
        kernel_meaning: "undo the last geometry edit",
        why: "There must be one undo stack, not two, or an undo means different \
              things depending on what you last did. pdf_core already routes \
              every mutation through a serialisable Command with a history, so \
              that stack is the one — and phase 4 must fold markup edits into \
              it rather than keeping the kernel's separate.",
        kernel_keeps: Some("u"),
    },
    Override {
        token: "redo",
        kernel_meaning: "redo the last undone geometry edit",
        why: "Same as `undo`.",
        kernel_keeps: Some("y"),
    },
    Override {
        token: "help",
        kernel_meaning: "list the kernel's commands",
        why: "One help, covering both namespaces. A help that listed only half \
              the words the box accepts would be worse than none.",
        kernel_keeps: Some("?"),
    },
];

/// A verb the kernel understands that Pagify declines, and why.
pub struct Refusal {
    pub token: &'static str,
    pub why: &'static str,
}

/// §6's "Leave" column and §10's exclusions, as tokens.
///
/// These parse perfectly well in the kernel. They are refused with an
/// explanation rather than run, because the failure otherwise is a user drawing
/// a wall on a drawing of a wall and wondering why nothing about it works.
pub const REFUSED: &[Refusal] = &[
    Refusal { token: "wall", why: "walls are architectural objects that exist to be promoted into 3D. On a PDF page, draw a polyline." },
    Refusal { token: "w", why: "`w` is the kernel's alias for `wall`. On a PDF page, draw a polyline." },
    Refusal { token: "wallstyle", why: "wall styles configure an object Pagify does not have." },
    Refusal { token: "wstyle", why: "wall styles configure an object Pagify does not have." },
    Refusal { token: "blockdiff", why: "comparing block definitions is a CAD workflow. Stamps are placed, not diffed." },
    Refusal { token: "bdiff", why: "comparing block definitions is a CAD workflow. Stamps are placed, not diffed." },
    Refusal { token: "units", why: "a PDF has no document unit system. Use `calibrate` to fix the page scale from two points and a known distance." },
    Refusal { token: "unit", why: "a PDF has no document unit system. Use `calibrate` to fix the page scale from two points and a known distance." },
    Refusal { token: "insunits", why: "a PDF has no document unit system. Use `calibrate` to fix the page scale from two points and a known distance." },
    Refusal { token: "scene", why: "there is no 3D here, by decision. See §10." },
    Refusal { token: "scenedump", why: "there is no 3D here, by decision. See §10." },
    Refusal { token: "3dstate", why: "there is no 3D here, by decision. See §10." },
];

/// Verbs that are Pagify's, are named, and are not built yet.
const PLANNED: &[(&str, &str, &str)] = &[
    ("comment", "a later phase", "threaded review comments"),

    // The ribbon, from here down. Every button on it runs a command, and a
    // button whose command was not a word would answer `unknown command` —
    // which reads as a broken button rather than as an unbuilt feature.
    //
    // Kept in step with `Tab::buttons` by `every_ribbon_button_runs_a_command`,
    // because two hand-maintained lists of a hundred and fifty names each will
    // not stay in step on their own.
    ("add3d", "the editing phase", "the Add 3D tool"),
    ("assistant", "the viewing phase", "the Assistant tool"),
    ("autotag", "the accessibility phase", "the Autotag Document tool"),
    ("calcorder", "the forms phase", "the Calculation Order tool"),
    ("clipboard", "the capture phase", "the Clipboard tool"),
    ("cloudstorage", "the sharing phase", "the Cloud Storage tool"),
    ("combfield", "the forms phase", "the Comb Field tool"),
    ("createform", "the forms phase", "the Form tool"),
    ("crossref", "the editing phase", "the Cross Reference tool"),
    ("editxfa", "the forms phase", "the Edit Static XFA Form tool"),
    ("email", "the sharing phase", "the Email tool"),
    ("emailattach", "the sharing phase", "the Attach to Email tool"),
    ("fieldbarcode", "the forms phase", "the Barcode tool"),
    ("fieldbutton", "the forms phase", "the Push Button tool"),
    ("fieldcheckbox", "the forms phase", "the Check Box tool"),
    ("fieldcombo", "the forms phase", "the Combo Box tool"),
    ("fielddate", "the forms phase", "the Date Field tool"),
    ("fieldimage", "the forms phase", "the Image Field tool"),
    ("fieldlist", "the forms phase", "the List Box tool"),
    ("fieldradio", "the forms phase", "the Radio Button tool"),
    ("fieldsignature", "the forms phase", "the Signature Field tool"),
    ("fieldtext", "the forms phase", "the Text Field tool"),
    ("formdesigner", "the forms phase", "the Designer Assistant tool"),
    ("formrecognise", "the forms phase", "the Run Form Field Recognition tool"),
    ("fromscanner", "the scanning phase", "the From Scanner tool"),
    ("javascript", "the forms phase", "the JavaScript tool"),
    ("media", "the editing phase", "the Audio & Video tool"),
    ("onlineform", "the signing phase", "the Create Online Form tool"),
    ("pagetemplates", "the forms phase", "the Page Templates tool"),
    ("portfolio", "the conversion phase", "the PDF Portfolio tool"),
    ("readaloud", "the viewing phase", "the Read tool"),
    ("requestsignature", "the signing phase", "the Request Signature tool"),
    ("sendbulk", "the signing phase", "the Send in Bulk tool"),
    ("sendreview", "the sharing phase", "the Send for Review tool"),
    ("sharelink", "the sharing phase", "the Share Link tool"),
    ("signbranding", "the signing phase", "the Add E-Sign Branding tool"),
    ("tooffice", "the conversion phase", "the To MS Office tool"),
    ("toolsettings", "the forms phase", "the Tool Settings tool"),
    ("toother", "the conversion phase", "the To Other tool"),
    ("trackreviews", "the sharing phase", "the Track Reviews tool"),
    ("transitions", "the viewing phase", "the Page Transitions tool"),
];

/// Parse a line as a Pagify verb.
///
/// The three-way return is the crux of two-namespace dispatch, and collapsing
/// it to two would lose the case that matters:
///
/// - `None` — not Pagify's word. The caller falls through to the kernel.
/// - `Some(Err(..))` — it *is* our word and the arguments are wrong. The caller
///   must report this rather than falling through, or `page banana` would be
///   answered with `unknown command 'page'`, which is a lie.
/// - `Some(Ok(..))` — ours, understood.
pub fn parse(line: &str) -> Option<Result<Verb, String>> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let head = tokens.first()?.to_ascii_lowercase();
    let rest = &tokens[1..];

    // Everything after the verb, with the spacing intact. Splitting a path on
    // whitespace truncates it at the first space, which on a Mac means most of
    // the Desktop and all of "My Documents" cannot be opened at all.
    let tail = line.trim().get(tokens[0].len()..).unwrap_or("").trim();

    if let Some((verb, phase, _)) = PLANNED.iter().find(|(v, _, _)| *v == head) {
        return Some(Ok(Verb::Planned { verb, phase }));
    }

    parse_locking_and_redaction(head.as_str(), rest, tail)
        .or_else(|| parse_fill_and_sign(head.as_str(), rest, tail))
        .or_else(|| parse_signatures(head.as_str(), rest, tail))
        .or_else(|| parse_security_options(head.as_str(), rest, tail))
        .or_else(|| parse_document_edits(head.as_str(), rest, tail))
        .or_else(|| parse_file_and_view(head.as_str(), rest, tail))
        .or_else(|| parse_conversion(head.as_str(), rest, tail))
        .or_else(|| parse_marks_and_misc(head.as_str(), rest, tail))
}
/// Resolve a path as a person typed it.
///
/// Three things go wrong otherwise, and all three make a file simply refuse to
/// open with no useful complaint:
///
/// - **Quotes.** Dragging a file into a terminal, or into this box, quotes it
///   when it has spaces. The quotes are part of the text, not the path.
/// - **`~`.** Nothing expands it but a shell, and there is no shell here.
/// - **Trailing whitespace**, which a paste or a drag leaves behind.
pub fn resolve_path(raw: &str) -> PathBuf {
    let mut text = raw.trim();

    for quote in ['"', '\''] {
        if text.len() >= 2 && text.starts_with(quote) && text.ends_with(quote) {
            text = &text[1..text.len() - 1];
            break;
        }
    }
    // A drag from Finder escapes spaces as `\ `.
    let text = text.replace("\\ ", " ");

    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if text == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(text)
}

/// Split "a path" from a trailing extra word, respecting quotes.
fn split_path_and_extra(tail: &str) -> Option<(String, String)> {
    let tail = tail.trim();
    if tail.is_empty() {
        return None;
    }

    // Quoted path: everything to the closing quote is the path.
    for quote in ['"', '\''] {
        if let Some(rest) = tail.strip_prefix(quote) {
            if let Some(end) = rest.find(quote) {
                return Some((rest[..end].to_string(), rest[end + 1..].trim().to_string()));
            }
        }
    }

    // Unquoted: a trailing token is an extra only if it looks like a page range
    // rather than part of a path. `1-3`, `all`, `2` — never anything with a
    // slash or a dot in it.
    match tail.rsplit_once(char::is_whitespace) {
        Some((path, last))
            if !last.contains('/')
                && !last.contains('.')
                && (last.eq_ignore_ascii_case("all")
                    || last.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ',')) =>
        {
            Some((path.trim().to_string(), last.to_string()))
        }
        _ => Some((tail.to_string(), String::new())),
    }
}

/// Every token this table claims, for the collision test and for help.
pub fn claimed_tokens() -> Vec<&'static str> {
    let mut tokens = vec![
        "open", "close", "save", "saveas", "page", "p", "next", "zoom", "z", "fit",
        "rotate", "undo", "redo", "pdfium", "version", "checkupdate", "quit", "exit", "help",
        "about", "shortcuts", "quickstart", "reportissue",
        "close!", "quit!", "exit!",
        // Several page operations are spelt out rather than taking the short
        // word: `delete`, `insert`, `scale`, `mp` and `dist` are all live
        // drawing commands in the kernel, and a PDF verb that shadowed them
        // would take away erase, block insert, the scale tool, match-properties
        // and distance from anyone who also uses SIMLUX. The guard test in
        // tests/command_box.rs is what caught every one of them.
        "extract", "import", "deletepage", "delpage", "insertpage", "movepage",
        "replacepage", "interleave", "toimage", "tohtml", "split", "wordcount", "blankdoc",
        "combine",
        "highlight", "hl", "note", "calibrate", "cal", "pagescale", "measure",
        "find", "findnext", "fn", "findprev", "fp", "replace", "spelling", "bookmark",
        "articlebox", "weblinks", "jointext", "reflow",
        "record", "stop", "endrecord", "replay", "pick", "textlayer", "whytext", "snapshot",
        "autoscroll", "ruler", "accesscheck", "accessreport", "autobookmarks", "preflight",
        "exportimages", "fromfiles", "viewsetting", "calculator", "managecomments",
        "pagemarks", "fromclipboard", "pencil", "rearrange", "fromtemplate", "addshape",
        "drawing", "keeptool", "formexport", "formimport", "formtosheet", "tooltip",
        "resetform", "flatten", "attach", "attachcomment", "tagspanel", "alttext", "readingorder", "viewsplit", "readingoptions", "reverseview", "stamp", "callout", "markredact",
        "searchhighlight", "compare", "changecolor",
        "outlinedfont", "lock", "lockall", "lockarea", "unlock", "secure",
        "hiddendata", "sanitize", "sanitise", "smartredact", "whiteout", "areahighlight", "sensitivity", "fillsign", "certify", "validate", "signature", "managesignatures", "applysignatures", "documentstatus", "status", "signrectangle", "signline", "signcheck", "signcross", "signdot", "predefinedtext", "moveobject", "editobject", "sessionlog",
        "tickmark", "inserttext", "replacetext", "addlink", "customstamp", "imageannotation",
        "textbox",
        "unsecure", "redact",
    ];
    tokens.extend(PLANNED.iter().map(|(v, _, _)| *v));
    tokens
}

/// Help, covering both namespaces — which is the whole reason `help` is one of
/// the deliberate overrides.
pub fn help_text(topic: Option<&str>) -> Vec<String> {
    if let Some(topic) = topic {
        let topic = topic.to_ascii_lowercase();

        if let Some(o) = DELIBERATE_OVERRIDES.iter().find(|o| o.token == topic) {
            let mut out = vec![format!("{topic} — Pagify's, deliberately.")];
            out.push(format!("  in SIMLUX this means: {}", o.kernel_meaning));
            out.push(format!("  here: {}", o.why));
            if let Some(alias) = o.kernel_keeps {
                out.push(format!("  the drawing command is still on `{alias}`."));
            }
            return out;
        }
        if let Some(r) = REFUSED.iter().find(|r| r.token == topic) {
            return vec![format!("{topic} — not in Pagify. {}", r.why)];
        }
        if let Some((v, phase, what)) = PLANNED.iter().find(|(v, _, _)| *v == topic) {
            return vec![format!("{v} — {what}. Planned for {phase}.")];
        }
        match topic.as_str() {
            "about" => return about_text(),
            "quickstart" => return quickstart_text(),
            "reportissue" => {
                return vec![format!("reportissue — a bug report goes to {ISSUES_URL}.")]
            }
            "shortcuts" => {
                return vec!["shortcuts — the window's key bindings, said as lines in the box.".into()]
            }
            _ => {}
        }
        return vec![format!("no help for '{topic}'.")];
    }

    vec![
        "document   open close save saveas quit".into(),
        "           close! / quit! discard unsaved marks — the plain forms refuse".into(),
        "view       page <n|next|prev|first|last>   zoom <percent|in|out|fit|width|actual>   rotate [90]".into(),
        "organize   extract <pages> <file>   import <file> [pages]   deletepage <pages>   insertpage   movepage <pages> <before>".into(),
        "review     highlight   note <text>".into(),
        "text       find <text>   findnext / fn   findprev / fp   — ⌘C copies a selection".into(),
        "           reflow — rebuild this page's reading order, when it does not read right".into(),
        "measure    calibrate <distance> [unit]   pagescale   measure <distance|area>".into(),
        "automate   record [name]   stop   replay <script.json>".into(),
        "edit       undo redo".into(),
        "text       textlayer   — what kind of text this page has, and why selection".into(),
        "           may be doing nothing".into(),
        "picking    pick <x,y>   — supplies a click to a waiting tool, so fillet and".into(),
        "           trim can be scripted and recorded as well as clicked".into(),
        "help       about   shortcuts   quickstart   reportissue".into(),
        "planned    comment — and the ribbon's unbuilt buttons, whose words name their phase".into(),
        "drawing    every SIMLUX command — line, polyline, trim, fillet, offset, … — with the same aliases".into(),
        "           `help <verb>` explains any word that means something different here.".into(),
        "".into(),
        "Page operations are spelt out — `deletepage`, `insertpage`, `pagescale` — because".into(),
        "`delete`, `insert` and `scale` are drawing commands and keep their SIMLUX meanings.".into(),
    ]
}

/// Where a bug report goes. Kept here so `about`, `reportissue` and the help
/// topic all say the same thing.
pub const ISSUES_URL: &str = "https://github.com/HSI-Lighting/Pagify/issues";

/// What `about` says — shared with `help about`, so the two cannot drift.
pub fn about_text() -> Vec<String> {
    vec![
        format!(
            "Pagify {} — a PDF editor whose whole interface is a command box: every ribbon \
             button runs a word you can type.",
            crate::VERSION
        ),
        "`help` lists every word, `shortcuts` the keys, and `reportissue` where a bug report goes."
            .into(),
    ]
}

/// What `quickstart` says — shared with `help quickstart`.
pub fn quickstart_text() -> Vec<String> {
    vec![
        "open — drag a PDF onto the window, or type `open` (⌘O).".into(),
        "type a word in the box and press Enter — `help` lists them all; the ribbon's buttons \
         type their own."
            .into(),
        "`page 3`, `next` and `prev` move; `+` and `-` zoom; ← → step pages.".into(),
        "drag over text to select it, `find <words>` searches, ⌘C copies the selection.".into(),
        "`reflow` rebuilds this page's reading order when selection or extraction reads wrong."
            .into(),
    ]
}


/// One domain of [`parse`]: the 12 commands below.
fn parse_locking_and_redaction(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "done" | "finish" => Ok(Verb::Finish),
        // Not "markredact": marking and applying are one gesture here, guarded
        // by a confirmation where anything is in the way rather than by a
        // separate mark-then-apply pass.
        "redact" => Ok(Verb::Redact),
        // Bare `lock` arms the rectangle tool. With pages named — `lock all`,
        // `lock 1-3` — it locks those pages whole, which is the only form that
        // can lock a document whose pages are scans or outlined type.
        "lock" => {
            if tail.is_empty() {
                Ok(Verb::Lock)
            } else {
                Ok(Verb::LockPages(tail.to_string()))
            }
        }
        "lockall" => Ok(Verb::LockPages("all".into())),
        "lockarea" => Ok(Verb::LockArea),
        "layers" => Ok(Verb::Layers),
        // Said the way every other program says it, because this is one of the
        // few gestures a reader arrives already knowing.
        "bringtofront" | "bringforward" | "tofront" => Ok(Verb::BringToFront),
        "sendtoback" | "sendbackward" | "toback" => Ok(Verb::SendToBack),
        "repairlocks" => Ok(Verb::RepairLocks),
        "opacity" | "transparency" => match tail.trim().trim_end_matches('%').parse::<f32>() {
            Ok(percent) if (0.0..=100.0).contains(&percent) => Ok(Verb::Opacity(percent)),
            Ok(_) => Err("opacity is between 0 and 100".into()),
            Err(_) => Err("opacity takes a percentage, 0 to 100 — `opacity 50`".into()),
        },
        "secure" => SecureOptions::parse(tail).map(Verb::Secure),
        "whiteout" => Ok(Verb::Whiteout),
        "areahighlight" => Ok(Verb::AreaHighlight),
        "pencil" => Ok(Verb::Pencil),
        _ => return None,
    })
}

/// One domain of [`parse`]: the 13 commands below.
fn parse_fill_and_sign(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "fill" => Ok(Verb::ToggleFill),
        "arrow" => Ok(Verb::DrawArrow),
        "signrectangle" => Ok(Verb::SignRectangle),
        "signline" => Ok(Verb::SignLine),
        // **Not `move`.** That word belongs to the drawing kernel, where it
        // moves marks and is tested doing so. Taking it would have broken a
        // tool that works in order to name one that did not exist yet.
        "moveobject" => Ok(Verb::MoveThing),
        "editobject" => Ok(Verb::EditObject),
        "predefinedtext" => Ok(Verb::PredefinedText({
            let tail = tail.trim();
            // Free text rather than sub-verbs: a snippet is whatever somebody
            // types, and reserving words like `list` out of it would mean a
            // snippet that happens to be one could never be kept.
            (!tail.is_empty()).then(|| tail.to_string())
        })),
        // The same three marks `fillsign` makes, one press away — the PagiSign
        // tab has a button for each, and each of them said "not built yet"
        // about a mark this program had been making since `fillsign` was wired.
        // Aliases rather than tools: one implementation, three doors.
        "signcheck" => Ok(Verb::FillSign(Some("tick".into()))),
        "signcross" => Ok(Verb::FillSign(Some("cross".into()))),
        "signdot" => Ok(Verb::FillSign(Some("dot".into()))),
        // The Comment tab's TickMark is the same tick the form-fill words
        // place: this app has one tick, and the button runs it.
        "tickmark" => Ok(Verb::FillSign(Some("tick".into()))),
        "documentstatus" | "status" => Ok(Verb::DocumentStatus),
        "sessionlog" => Ok(Verb::SessionLog),
        "applysignatures" => Ok(Verb::ApplySignatures),
        _ => return None,
    })
}

/// One domain of [`parse`]: the 2 commands below.
fn parse_signatures(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "managesignatures" => {
            let tail = tail.trim();
            let (word, rest) = match tail.split_once(char::is_whitespace) {
                Some((word, rest)) => (word.to_ascii_lowercase(), rest.trim()),
                None => (tail.to_ascii_lowercase(), ""),
            };
            match (word.as_str(), rest) {
                ("" | "open" | "show", "") => Ok(Verb::ManageSignatures(Signatures::Open)),
                ("list", "") => Ok(Verb::ManageSignatures(Signatures::List)),
                ("use" | "choose", name) if !name.is_empty() => {
                    Ok(Verb::ManageSignatures(Signatures::Use(name.to_string())))
                }
                ("delete" | "remove" | "forget", name) if !name.is_empty() => {
                    Ok(Verb::ManageSignatures(Signatures::Forget(name.to_string())))
                }
                ("rename" | "call", name) if !name.is_empty() => {
                    Ok(Verb::ManageSignatures(Signatures::Rename(name.to_string())))
                }
                // Named without a name — the mistake worth its own sentence,
                // because the fix is a word away rather than a different verb.
                ("use" | "choose" | "delete" | "remove" | "forget" | "rename" | "call", "") => {
                    Err(format!("managesignatures {word}: which one? `managesignatures list` says."))
                }
                _ => Err(format!(
                    "managesignatures: don't know {tail:?} — try list, use <name>, \
                     rename <name>, or delete <name>"
                )),
            }
        }
        "signature" => {
            let tail = tail.trim();
            let (head, rest) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
            match head.to_ascii_lowercase().as_str() {
                "" | "place" | "put" | "sign" => Ok(Verb::Signature(SignatureAction::Place)),
                "draw" | "new" | "create" => Ok(Verb::Signature(SignatureAction::Draw)),
                "upload" | "image" | "file" => {
                    let rest = rest.trim();
                    if rest.is_empty() {
                        Ok(Verb::Signature(SignatureAction::Upload(None)))
                    } else {
                        Ok(Verb::Signature(SignatureAction::Upload(Some(resolve_path(rest)))))
                    }
                }
                other => Err(format!(
                    "signature: don't know {other:?} — `signature` places the one you \
                     have, `signature draw` makes a new one by hand, `signature upload` \
                     adds one from a picture file"
                )),
            }
        }
        _ => return None,
    })
}

/// One domain of [`parse`]: the 8 commands below.
fn parse_security_options(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "validate" => Ok(Verb::Validate),
        "certify" | "sign" => {
            let tail = tail.trim();
            if tail.is_empty() {
                Ok(Verb::Certify(None))
            } else {
                Ok(Verb::Certify(Some(resolve_path(tail))))
            }
        }
        "fillsign" => {
            let tail = tail.trim();
            if tail.is_empty() {
                Ok(Verb::FillSign(None))
            } else if pdf_core::document::FillMark::parse(tail).is_some() {
                Ok(Verb::FillSign(Some(tail.to_string())))
            } else if matches!(tail.to_ascii_lowercase().as_str(), "line" | "rule" | "strike") {
                // As with the box: two points rather than one, so its own verb,
                // and reachable through the tool its siblings live on.
                Ok(Verb::SignLine)
            } else if matches!(
                tail.to_ascii_lowercase().as_str(),
                "rectangle" | "box" | "rect"
            ) {
                // The fifth mark, which takes two corners rather than a point —
                // so it is its own verb. Answered here rather than refused,
                // because somebody reaching for it through `fillsign` is asking
                // for exactly the right thing.
                Ok(Verb::SignRectangle)
            } else {
                Err(format!(
                    "fillsign: don't know {tail:?} — `fillsign` types where you \
                     click, or try tick, cross, dot, line or rectangle"
                ))
            }
        }
        "sensitivity" => {
            let tail = tail.trim();
            if tail.is_empty() {
                Ok(Verb::Sensitivity(None))
            } else if tail.eq_ignore_ascii_case("none") || tail.eq_ignore_ascii_case("clear") {
                Ok(Verb::Sensitivity(Some(String::new())))
            } else if pdf_core::document::sensitivity::Sensitivity::parse(tail).is_some() {
                Ok(Verb::Sensitivity(Some(tail.to_string())))
            } else {
                Err(format!(
                    "sensitivity: don't know {tail:?} — try public, internal, \
                     confidential, secret, or none"
                ))
            }
        }
        "smartredact" => match tail.trim().to_ascii_lowercase().as_str() {
            "" | "find" | "scan" => Ok(Verb::SmartRedact { redact: false }),
            "redact" | "all" | "apply" => Ok(Verb::SmartRedact { redact: true }),
            other => Err(format!(
                "smartredact: don't know {other:?} — `smartredact` reports what it \
                 finds, `smartredact redact` blacks it out"
            )),
        },
        "hiddendata" | "sanitize" | "sanitise" => match tail.trim().to_ascii_lowercase().as_str() {
            "" => Ok(Verb::HiddenData { clean: false }),
            "clean" | "remove" | "strip" => Ok(Verb::HiddenData { clean: true }),
            other => Err(format!(
                "hiddendata: don't know {other:?} — `hiddendata` reports what is there, \
                 `hiddendata clean` takes it out"
            )),
        },
        "unsecure" => Ok(Verb::Unsecure),
        "unlock" => Ok(Verb::Unlock),
        _ => return None,
    })
}

/// One domain of [`parse`]: the 19 commands below.
fn parse_document_edits(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "copy" | "copytext" => Ok(Verb::CopyText),
        "paste" => Ok(Verb::Paste),
        "reversepages" => Ok(Verb::ReversePages),
        "thumbnails" => Ok(Verb::Thumbnails),
        // Rearrange, in Acrobat's Organize tab, is the page grid you drag
        // pages around in — which is this app's Organize grid, so the word
        // opens it.
        "rearrange" => Ok(Verb::Thumbnails),
        "appearance" => Ok(Verb::ToggleAppearance),
        // An inset rather than a rectangle. Trimming the same amount off every
        // edge is what "crop the margins" means, and a rectangle typed into a
        // box is four numbers nobody can picture.
        "edittext" => Ok(Verb::EditText),
        "addtext" | "typewriter" => Ok(Verb::AddText(tail.to_string())),
        // Insert Text, in Acrobat's Comment tab, is text laid on the page at
        // a click — which is what this app's addtext writes: an annotation
        // in the app's own text encoding, not page content.
        "inserttext" => Ok(Verb::AddText(tail.to_string())),
        // Replace Text is the edit-the-run operation this app has: the
        // replacement is typed into the editor `edittext` opens, so the run
        // is rewritten rather than covered over.
        "replacetext" => Ok(Verb::EditText),
        // The Comment tab's Textbox is the drag-a-box-and-type flow bare
        // `addtext` arms — the words are typed in the box rather than
        // covered over, and land as this app's written text. No frame is
        // drawn around them: the annotation frames this build declares are
        // not filled yet, and a box drawn by guessing at the line's width
        // would misplace itself.
        "textbox" => Ok(Verb::AddText(tail.to_string())),
        "addimage" => {
            let tail = tail.trim();
            if tail.is_empty() {
                Ok(Verb::AddImage(None))
            } else {
                Ok(Verb::AddImage(Some(resolve_path(tail))))
            }
        }
        // A custom stamp, and a comment that carries a picture, are both a
        // picture placed where it is clicked — which is what `addimage`
        // does. This app keeps no reusable stamp library, so the picture is
        // placed and then adjusted in place; the words exist so the
        // Comment tab's buttons run the operation they name.
        "customstamp" | "imageannotation" => {
            let tail = tail.trim();
            if tail.is_empty() {
                Ok(Verb::AddImage(None))
            } else {
                Ok(Verb::AddImage(Some(resolve_path(tail))))
            }
        }
        "viewsingle" => Ok(Verb::SetLayout(crate::reader::Layout::Single)),
        "viewfacing" => Ok(Verb::SetLayout(crate::reader::Layout::Facing)),
        "viewcover" => Ok(Verb::SetLayout(crate::reader::Layout::FacingWithCover)),
        // Continuous, as Acrobat means it, is this app's *default shape*: the
        // strip already scrolls through every row, so "continuous single" and
        // "continuous facing" are the single and facing layouts themselves.
        // Wired to those rather than left as placeholders, because the
        // behaviour the words ask for is real; if a page-snapped mode is ever
        // added, these two words are where its switch belongs.
        "viewcontinuous" => Ok(Verb::SetLayout(crate::reader::Layout::Single)),
        "viewcontinuousfacing" => Ok(Verb::SetLayout(crate::reader::Layout::Facing)),
        "resizepages" => {
            let pages = rest.first().copied().unwrap_or("all").to_string();
            match rest.get(1).and_then(|n| paper_size(n)) {
                Some((width_pt, height_pt)) => {
                    Ok(Verb::ResizePages { pages, width_pt, height_pt })
                }
                None => Err(
                    "resizepages: a size, as in `resizepages all a4` — or `612x792` in points."
                        .into(),
                ),
            }
        }
        "croppages" => {
            let pages = rest.first().copied().unwrap_or("all").to_string();
            match rest.get(1).map(|m| m.parse::<f32>()) {
                Some(Ok(margin)) if margin > 0.0 => Ok(Verb::CropPages { pages, margin }),
                _ => Err("croppages: pages and an inset in points, as in `croppages all 36`.".into()),
            }
        }
        "duplicatepage" => {
            Ok(Verb::DuplicatePages(rest.first().copied().unwrap_or("").to_string()))
        }
        "swappages" => match (rest.first(), rest.get(1)) {
            (Some(a), Some(b)) => match (a.parse::<usize>(), b.parse::<usize>()) {
                (Ok(a), Ok(b)) => Ok(Verb::SwapPages { a, b }),
                _ => Err("swappages: two page numbers, as in `swappages 2 5`.".into()),
            },
            // Named rather than silently defaulted: guessing which two pages
            // somebody meant is not a service.
            _ => Err("swappages: two page numbers, as in `swappages 2 5`.".into()),
        },
        "rotatepages" => {
            let pages = rest.first().copied().unwrap_or("all").to_string();
            let quarters = match rest.get(1) {
                None => 1,
                Some(q) => match q.parse::<i32>() {
                    Ok(n) => n,
                    Err(_) => {
                        return Some(Err(
                            "rotatepages: quarter-turns, as in `rotatepages all 1`.".into()
                        ))
                    }
                },
            };
            Ok(Verb::RotatePages { pages, quarters })
        }
        // A page range, defaulting to the one being looked at. `extracttext
        // all` is the whole document, which is what a scan wants.
        "extracttext" | "extract-text" => {
            Ok(Verb::ExtractText(rest.first().copied().unwrap_or("").to_string()))
        }
        // No path is a request for the file picker, exactly as `open` treats
        // it. `remove` takes a path rather than an index because a typed or
        // recorded `outlinedfont remove <path>` should still work once the
        // list has moved on since it was written.
        "outlinedfont" => {
            let first_word = tail.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
            if tail.is_empty() {
                Ok(Verb::OutlinedFont(OutlinedFontAction::Dialog))
            } else if first_word == "clear" {
                Ok(Verb::OutlinedFont(OutlinedFontAction::Clear))
            } else if first_word == "remove" {
                let path = tail.split_once(char::is_whitespace).map(|(_, rest)| rest.trim()).unwrap_or("");
                if path.is_empty() {
                    Err("usage: outlinedfont remove <path>".into())
                } else {
                    Ok(Verb::OutlinedFont(OutlinedFontAction::Remove(resolve_path(path))))
                }
            } else {
                Ok(Verb::OutlinedFont(OutlinedFontAction::Add(resolve_path(tail))))
            }
        }
        "clearhistory" | "forgethistory" => Ok(Verb::ClearHistory),
        _ => return None,
    })
}

/// One domain of [`parse`]: the 17 commands below.
fn parse_file_and_view(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "selecttool" => Ok(Verb::Pointer(PointerMode::Select)),
        "hand" | "pan" => Ok(Verb::Pointer(PointerMode::Pan)),
        "open" => {
            if tail.is_empty() {
                // No path is not an error: it is a request for the file picker,
                // which is what someone typing `open` almost always wants.
                Ok(Verb::OpenDialog)
            } else {
                Ok(Verb::Open(resolve_path(tail)))
            }
        }
        "close" => Ok(Verb::Close { force: false }),
        "close!" => Ok(Verb::Close { force: true }),
        "save" => Ok(Verb::Save),
        "saveas" => {
            if tail.is_empty() {
                // No path is not an error: it is a request for the file
                // picker, the same as bare `open` — see `Verb::OpenDialog`.
                Ok(Verb::SaveAsDialog)
            } else {
                Ok(Verb::SaveAs(resolve_path(tail)))
            }
        }

        "page" | "p" => match rest.first().map(|s| s.to_ascii_lowercase()) {
            None => Err("usage: page <n|next|prev|first|last>".into()),
            Some(word) => match word.as_str() {
                "next" | "n" => Ok(Verb::Page(PageTarget::Next)),
                "prev" | "previous" | "p" => Ok(Verb::Page(PageTarget::Previous)),
                "first" => Ok(Verb::Page(PageTarget::First)),
                "last" => Ok(Verb::Page(PageTarget::Last)),
                number => match number.parse::<usize>() {
                    Ok(0) => Err("pages are numbered from 1".into()),
                    Ok(n) => Ok(Verb::Page(PageTarget::Number(n))),
                    Err(_) => Err(format!("page: expected a number or next/prev/first/last, got '{number}'")),
                },
            },
        },
        "next" => Ok(Verb::Page(PageTarget::Next)),

        "zoom" | "z" => match rest.first().map(|s| s.to_ascii_lowercase()) {
            None => Err("usage: zoom <percent|in|out|fit|width|actual>".into()),
            Some(word) => match word.as_str() {
                "in" => Ok(Verb::Zoom(ZoomTarget::In)),
                "out" => Ok(Verb::Zoom(ZoomTarget::Out)),
                "fit" => Ok(Verb::Zoom(ZoomTarget::Fit)),
                "width" => Ok(Verb::Zoom(ZoomTarget::Width)),
                "actual" | "1" | "100" => Ok(Verb::Zoom(ZoomTarget::Actual)),
                number => match number.trim_end_matches('%').parse::<f32>() {
                    Ok(pct) if pct > 0.0 => Ok(Verb::Zoom(ZoomTarget::Factor(pct / 100.0))),
                    Ok(_) => Err("zoom: a percentage must be greater than zero".into()),
                    Err(_) => Err(format!("zoom: expected a percentage or in/out/fit/width/actual, got '{number}'")),
                },
            },
        },
        "fit" => Ok(Verb::Zoom(ZoomTarget::Fit)),

        "rotate" => match rest.first() {
            None => Ok(Verb::RotatePage(90)),
            Some(word) => match word.parse::<i32>() {
                Ok(deg) if deg.rem_euclid(90) == 0 => Ok(Verb::RotatePage(deg.rem_euclid(360))),
                Ok(_) => Err("rotate: a page turns in quarters — 90, 180, 270 or -90".into()),
                Err(_) => Err(format!("rotate: expected an angle, got '{word}'")),
            },
        },

        "extract" => match (rest.first(), rest.get(1)) {
            (Some(pages), Some(dest)) => Ok(Verb::Extract {
                pages: (*pages).to_string(),
                dest: PathBuf::from(dest),
            }),
            _ => Err("usage: extract <pages> <path.pdf>   e.g. extract 1-3,7 chapter.pdf".into()),
        },
        // `import` takes a path *and* an optional page range, so the path is
        // the tail minus a trailing range token — quoted if it has spaces.
        "import" => match split_path_and_extra(tail) {
            Some((path, extra)) => Ok(Verb::Import {
                source: resolve_path(&path),
                pages: if extra.is_empty() { "all".into() } else { extra },
            }),
            None => Err("usage: import <path.pdf> [pages]".into()),
        },
        // `delete` / `del` are the kernel's, and erase geometry. A page
        // delete has to be spelt out — see the note in claimed_tokens().
        "deletepage" | "delpage" => match rest.first() {
            Some(pages) => Ok(Verb::DeletePages((*pages).to_string())),
            None => Err("usage: deletepage <pages>   e.g. deletepage 4, or deletepage 2-5".into()),
        },
        "insertpage" => Ok(Verb::InsertPage),
        "movepage" => match (rest.first(), rest.get(1)) {
            (Some(pages), Some(before)) => match before.parse::<usize>() {
                Ok(0) => Err("pages are numbered from 1".into()),
                Ok(n) => Ok(Verb::MovePages { pages: (*pages).to_string(), before: n }),
                Err(_) => Err(format!("movepage: '{before}' is not a page number")),
            },
            _ => Err("usage: movepage <pages> <before>   e.g. movepage 5 2".into()),
        },
        // Replace takes a path *and* an optional page range, like `import` —
        // the path is the tail minus a trailing range token. Bare, it is a
        // request for the file picker, and the source's first page is what
        // stands in for the one in view.
        "replacepage" => match split_path_and_extra(tail) {
            Some((path, extra)) => Ok(Verb::ReplacePage {
                source: Some(resolve_path(&path)),
                pages: if extra.is_empty() { "1".into() } else { extra },
            }),
            None => Ok(Verb::ReplacePage { source: None, pages: "1".into() }),
        },
        // Interleave reads the same way; with nothing said, every page of the
        // source takes its turn.
        "interleave" => match split_path_and_extra(tail) {
            Some((path, extra)) => Ok(Verb::Interleave {
                source: Some(resolve_path(&path)),
                pages: if extra.is_empty() { "all".into() } else { extra },
            }),
            None => Ok(Verb::Interleave { source: None, pages: "all".into() }),
        },
        // Split takes an optional count first; everything after it is the
        // folder, quoted if it has spaces.
        "split" => {
            let mut words = tail.splitn(2, char::is_whitespace);
            let first = words.next().unwrap_or("");
            let rest = words.next().unwrap_or("").trim();
            if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) {
                match first.parse::<usize>() {
                    Ok(0) => return Some(Err("split: pages per file must be at least 1".into())),
                    Ok(every) => Ok(Verb::Split {
                        every,
                        dest: (!rest.is_empty()).then(|| resolve_path(rest)),
                    }),
                    // Not a count after all — a very long digit string is a
                    // path the same way any other word is.
                    Err(_) => Ok(Verb::Split { every: 1, dest: Some(resolve_path(tail)) }),
                }
            } else {
                Ok(Verb::Split { every: 1, dest: (!tail.is_empty()).then(|| resolve_path(tail)) })
            }
        }
        "wordcount" => Ok(Verb::WordCount { pages: tail.to_string() }),
        // Page marks: a corner, then the words. Only the left corners are
        // offered, and that is a measurement decision rather than a taste
        // one: the engine lays a written line out from its own left edge,
        // and this build has no way to ask how wide the line will be, so a
        // centred or right-aligned mark could only be placed by guessing.
        "pagemarks" => {
            let mut words = tail.trim().splitn(2, char::is_whitespace);
            let first = words.next().unwrap_or("");
            let rest = words.next().unwrap_or("").trim();
            let (top, text) = match first.to_ascii_lowercase().as_str() {
                "topleft" | "top" => (true, rest.to_string()),
                "bottomleft" | "bottom" => (false, rest.to_string()),
                "topright" | "bottomright" | "topcenter" | "bottomcenter" | "centre"
                | "center" => {
                    return Some(Err(
                        "pagemarks: only `topleft` and `bottomleft` — the other corners need \
                         the line measured, which this build does not do"
                            .into(),
                    ))
                }
                // No position word: everything said is the text.
                _ => (false, tail.trim().to_string()),
            };
            if text.is_empty() {
                return Some(Err(
                    "pagemarks: the words to write, as in `pagemarks Page {n}`".into(),
                ));
            }
            Ok(Verb::PageMarks { top, text })
        }
        "fromclipboard" => Ok(Verb::FromClipboard),
        // Add Shapes (Edit tab) and Drawing (Comment tab) are both answered
        // with the shape words this build has: some kinds are the engine's
        // own drawing words and some are Pagify's, and which is which is not
        // something a button should make anyone learn — the answer names
        // them all, and the freehand route besides.
        "addshape" | "drawing" => Ok(Verb::ShapeWords),
        "keeptool" => Ok(Verb::KeepTool),
        _ => return None,
    })
}

/// One domain of [`parse`]: the conversion phase's words — blank paper, a
/// copy of a template, and the form data going in and out.
fn parse_conversion(head: &str, _rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        // Words that make a document: blank paper, a copy of a template,
        // pasted words, and the form data going in and out.
        "fromtemplate" => Ok(Verb::FromTemplate {
            source: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "formexport" => Ok(Verb::FormExport {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "formimport" => Ok(Verb::FormImport {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "tagspanel" => Ok(Verb::TagSpanel),
        "alttext" => Ok(Verb::AltText { pages: tail.trim().to_string() }),
        "readingorder" => Ok(Verb::ReadingOrder { pages: tail.trim().to_string() }),
        "viewsplit" => Ok(Verb::ViewSplit),
        "readingoptions" => Ok(Verb::ReadingOptions),
        "reverseview" => Ok(Verb::ReverseView),
        "markredact" => Ok(Verb::MarkRedact),
        "stamp" => {
            let words = tail.trim();
            if words.is_empty() {
                return Some(Err("stamp: the words to stamp, as in `stamp Approved`".into()));
            }
            Ok(Verb::Stamp { words: words.to_string() })
        }
        "callout" => {
            let words = tail.trim();
            if words.is_empty() {
                return Some(Err(
                    "callout: the words for the box, as in `callout see figure 2`".into(),
                ));
            }
            Ok(Verb::Callout { words: words.to_string() })
        }
        "flatten" => Ok(Verb::Flatten),
        "attach" => Ok(Verb::Attach {
            source: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "attachcomment" => Ok(Verb::AttachComment {
            source: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "formtosheet" => Ok(Verb::FormToSheet {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "resetform" => Ok(Verb::ResetForm),
        // A field's name never has spaces in it, so the first word is the
        // field and everything after it is the tooltip.
        "tooltip" => {
            let mut words = tail.trim().splitn(2, char::is_whitespace);
            let field = words.next().unwrap_or("");
            let text = words.next().unwrap_or("").trim();
            if field.is_empty() || text.is_empty() {
                return Some(Err(
                    "tooltip: a field and its words, as in `tooltip email Your address`"
                        .into(),
                ));
            }
            Ok(Verb::ToolTip { field: field.to_string(), text: text.to_string() })
        }
        // No arguments means "ask me where", the same shape `open`/`saveas`
        // already have.
        "toimage" => Ok(Verb::ToImage {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "tohtml" => Ok(Verb::ToHtml {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        // Blank paper: an optional size and an optional ruling, in either
        // order and both optional, because `blankdoc lined` and `blankdoc a4`
        // are both things a person says.
        "blankdoc" => {
            let mut width_pt = 612.0f32;
            let mut height_pt = 792.0f32;
            let mut ruling = Ruling::None;
            for word in tail.split_whitespace() {
                match word.to_ascii_lowercase().as_str() {
                    "a4" => {
                        width_pt = 595.0;
                        height_pt = 842.0;
                    }
                    "a3" => {
                        width_pt = 842.0;
                        height_pt = 1191.0;
                    }
                    "letter" => {
                        width_pt = 612.0;
                        height_pt = 792.0;
                    }
                    "legal" => {
                        width_pt = 612.0;
                        height_pt = 1008.0;
                    }
                    "lined" => ruling = Ruling::Lined,
                    "grid" | "squared" => ruling = Ruling::Grid,
                    "dots" | "dotted" => ruling = Ruling::Dots,
                    "plain" => ruling = Ruling::None,
                    other => {
                        return Some(Err(format!(
                            "blankdoc: don't know {other:?} — try a size (a4, a3, letter, \
                             legal) or a ruling (lined, grid, dots)"
                        )))
                    }
                }
            }
            Ok(Verb::BlankDoc { width_pt, height_pt, ruling })
        }
        // A folder to take the PDFs from; bare is a request for the picker.
        "combine" => Ok(Verb::Combine {
            folder: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        _ => return None,
    })
}

/// One domain of [`parse`]: the 35 commands below.
fn parse_marks_and_misc(head: &str, rest: &[&str], tail: &str) -> Option<Result<Verb, String>> {
    Some(match head {
        "highlight" | "hl" => Ok(Verb::MarkText(Markup::Highlight)),
        "marks" | "annotations" => Ok(Verb::ListMarks),
        "removemark" => match rest.first() {
            Some(n) => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(Verb::RemoveMark(n)),
                _ => Err("removemark: a number from `marks`, as in `removemark 2`.".into()),
            },
            None => Err("removemark: a number from `marks`, as in `removemark 2`.".into()),
        },
        "underline" => Ok(Verb::MarkText(Markup::Underline)),
        "strikeout" => Ok(Verb::MarkText(Markup::StrikeOut)),
        "squiggly" => Ok(Verb::MarkText(Markup::Squiggly)),
        // Not aliased to `f`: that is the kernel's `fillet`, and `f 25` would
        // have become a search for "25". The guard test did not catch it,
        // because it only exercised bare aliases and a bare `f` falls through
        // correctly — see `an_alias_with_arguments_still_reaches_the_kernel`.
        "find" if !tail.is_empty() => Ok(Verb::Find(tail.to_string())),
        "find" => Err("usage: find <text>".into()),
        "findnext" | "fn" => Ok(Verb::FindStep { forward: true }),
        "findprev" | "fp" => Ok(Verb::FindStep { forward: false }),
        "replace" => Ok(Verb::Replace),
        "spelling" => Ok(Verb::Spelling),
        "bookmark" => Ok(Verb::Bookmark),
        "articlebox" => Ok(Verb::ArticleBox),
        "weblinks" => Ok(Verb::Weblinks),
        // The Edit tab's Link tool: a selection and an address — the same
        // workflow `weblinks` runs.
        "addlink" => Ok(Verb::Weblinks),
        "jointext" => Ok(Verb::JoinText),
        "matchproperties" => Ok(Verb::MatchProperties),
        "reflow" => Ok(Verb::Reflow),
        "note" => {
            let text = rest.join(" ");
            if text.trim().is_empty() {
                Err("usage: note <what it says>".into())
            } else {
                Ok(Verb::Note(text))
            }
        }

        "calibrate" | "cal" => match (rest.first(), rest.get(1)) {
            (Some(distance), unit) => match distance.parse::<f64>() {
                Ok(d) if d > 0.0 && d.is_finite() => Ok(Verb::Calibrate {
                    distance: d,
                    unit: unit.map(|u| (*u).to_string()).unwrap_or_else(|| "m".into()),
                }),
                Ok(_) => Err("calibrate: the real distance must be a positive number".into()),
                Err(_) => Err(format!("calibrate: '{distance}' is not a distance")),
            },
            _ => Err("usage: calibrate <real distance> [unit]   then pick two points".into()),
        },
        "pagescale" => Ok(Verb::Scale),
        "measure" => match rest.first().map(|s| s.to_ascii_lowercase()).as_deref() {
            None | Some("distance") | Some("d") => Ok(Verb::Measure(MeasureKind::Distance)),
            Some("area") | Some("a") => Ok(Verb::Measure(MeasureKind::Area)),
            Some(other) => Err(format!("measure: expected distance or area, got '{other}'")),
        },

        "record" => Ok(Verb::Record(
            rest.join(" ").trim().to_string(),
        )),
        "stop" | "endrecord" => Ok(Verb::StopRecording),
        "replay" => {
            if tail.is_empty() {
                Err("usage: replay <script.json>".into())
            } else {
                Ok(Verb::Replay(resolve_path(tail)))
            }
        }

        "pick" => match rest.first() {
            Some(pair) => match pair.split_once(',') {
                Some((x, y)) => match (x.trim().parse::<f64>(), y.trim().parse::<f64>()) {
                    (Ok(x), Ok(y)) => Ok(Verb::Pick(AppPointArg { x, y })),
                    _ => Err(format!("pick: expected x,y — got '{pair}'")),
                },
                None => Err(format!("pick: expected x,y — got '{pair}'")),
            },
            None => Err("usage: pick <x,y>   supplies the next click to a waiting command".into()),
        },

        "undo" => Ok(Verb::Undo),
        "redo" => Ok(Verb::Redo),
        "pdfium" => Ok(Verb::Pdfium),
        "version" => Ok(Verb::Version),
        "checkupdate" => Ok(Verb::CheckUpdate),
        "about" => Ok(Verb::About),
        "shortcuts" => Ok(Verb::Shortcuts),
        "quickstart" => Ok(Verb::QuickStart),
        "reportissue" => Ok(Verb::ReportIssue),
        "textlayer" | "whytext" => Ok(Verb::TextLayer),
        "snapshot" => Ok(Verb::Snapshot),
        "autoscroll" => Ok(Verb::AutoScroll),
        "ruler" => Ok(Verb::Ruler),
        "accesscheck" => Ok(Verb::AccessCheck),
        "accessreport" => Ok(Verb::AccessReport {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "autobookmarks" => Ok(Verb::AutoBookmarks),
        "preflight" => Ok(Verb::Preflight),
        "exportimages" => Ok(Verb::ExportAllImages {
            dest: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "fromfiles" => Ok(Verb::FromFiles {
            folder: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "viewsetting" => Ok(Verb::ViewSetting),
        "calculator" => Ok(Verb::Calculator),
        "managecomments" => Ok(Verb::ManageComments),
        "searchhighlight" => Ok(Verb::SearchHighlight { words: tail.trim().to_string() }),
        "compare" => Ok(Verb::Compare {
            source: (!tail.is_empty()).then(|| resolve_path(tail)),
        }),
        "changecolor" => match tail.trim().to_ascii_lowercase().as_str() {
            "" => Ok(Verb::ChangeColor(None)),
            "white" | "off" => Ok(Verb::ChangeColor(Some(PaperTint::White))),
            "grey" | "gray" => Ok(Verb::ChangeColor(Some(PaperTint::Grey))),
            "sepia" => Ok(Verb::ChangeColor(Some(PaperTint::Sepia))),
            "dark" => Ok(Verb::ChangeColor(Some(PaperTint::Dark))),
            other => Err(format!("changecolor: {other:?} — try white, grey, sepia or dark")),
        },
        "quit" | "exit" => Ok(Verb::Quit { force: false }),
        "quit!" | "exit!" => Ok(Verb::Quit { force: true }),
        "help" => Ok(Verb::Help(rest.first().map(|s| s.to_string()))),
        _ => return None,
    })
}

#[cfg(test)]
mod parse_domain_tests {
    use super::parse;

    /// Every command word the domain parsers claim, read out of this file's
    /// own source.
    fn claimed_words() -> Vec<(String, String)> {
        let src = include_str!("verbs.rs");
        let mut claims = Vec::new();
        let mut current: Option<String> = None;
        for line in src.lines() {
            if let Some(name) = line.strip_prefix("fn parse_") {
                current = Some(name.split('(').next().unwrap_or(name).to_string());
            } else if line.starts_with("fn ") || line.starts_with("pub fn ") {
                current = None;
            }
            let Some(domain) = &current else { continue };
            let Some(rest) = line.strip_prefix("        ") else { continue };
            if !rest.starts_with('"') {
                continue;
            }
            let Some(pattern) = rest.split("=>").next() else { continue };
            for atom in pattern.split('|') {
                let atom = atom.trim();
                let Some(open) = atom.find('"') else { continue };
                let Some(close) = atom[open + 1..].find('"') else { continue };
                claims.push((atom[open + 1..open + 1 + close].to_string(), domain.clone()));
            }
        }
        claims
    }

    /// **A command word must belong to exactly one domain parser.**
    ///
    /// When `parse` was one `match`, a word registered twice was a hard
    /// compile error — the second arm was unreachable and the compiler said
    /// so. Split into eight parsers that is no longer true: the second
    /// registration simply never runs, silently. This reads the source and
    /// re-establishes the guarantee by hand, and also checks that every word
    /// a parser claims is one `parse` actually reaches (a typo in a literal
    /// would otherwise be a command that exists in the completion table and
    /// nowhere else).
    #[test]
    fn no_command_is_claimed_by_two_domain_parsers() {
        let claims = claimed_words();
        assert!(
            claims.len() > 100,
            "the source scan found only {} commands — it has stopped reading the parsers: {claims:?}",
            claims.len()
        );

        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (word, domain) in &claims {
            if let Some(previous) = seen.insert(word, domain) {
                assert_eq!(
                    previous, domain,
                    "`{word}` is claimed by both `{previous}` and `{domain}` — whichever parser runs first wins silently"
                );
            }
        }

        for (word, domain) in &claims {
            assert!(
                parse(word).is_some(),
                "`{word}` is claimed by `{domain}` but `parse` returns None for it"
            );
        }
    }
}

#[cfg(test)]
mod page_phase_tests {
    use super::*;

    /// The page phase's two words, as the box reads them: a path and an
    /// optional range, bare forms asking for the file picker.
    #[test]
    fn the_page_phase_words_parse() {
        assert_eq!(
            parse("replacepage"),
            Some(Ok(Verb::ReplacePage { source: None, pages: "1".into() }))
        );
        assert_eq!(
            parse("replacepage other.pdf 2-3"),
            Some(Ok(Verb::ReplacePage { source: Some("other.pdf".into()), pages: "2-3".into() }))
        );
        assert_eq!(parse("interleave"), Some(Ok(Verb::Interleave { source: None, pages: "all".into() })));
        assert_eq!(
            parse("interleave /tmp/other.pdf"),
            Some(Ok(Verb::Interleave {
                source: Some("/tmp/other.pdf".into()),
                pages: "all".into()
            }))
        );
        for word in ["replacepage", "interleave"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod documentation_tests {
    use super::*;

    /// **The documentation phase's words are built, not planned.** The
    /// ribbon's five Help-tab buttons used to answer "planned for the
    /// documentation phase"; each now runs a real verb. Removing a word from
    /// `PLANNED` without teaching `parse` it is exactly the failure this
    /// catches.
    #[test]
    fn the_documentation_words_parse_to_their_verbs() {
        assert_eq!(parse("about"), Some(Ok(Verb::About)));
        assert_eq!(parse("shortcuts"), Some(Ok(Verb::Shortcuts)));
        assert_eq!(parse("quickstart"), Some(Ok(Verb::QuickStart)));
        assert_eq!(parse("reportissue"), Some(Ok(Verb::ReportIssue)));
        assert_eq!(parse("checkupdate"), Some(Ok(Verb::CheckUpdate)));
    }

    #[test]
    fn the_documentation_words_are_no_longer_planned() {
        for word in ["about", "shortcuts", "quickstart", "reportissue", "checkupdates"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
        // The old button's spelling was the placeholder's, never a command.
        assert!(parse("checkupdates").is_none(), "`checkupdates` should not parse");
    }

    /// What the words say, and that `help <word>` says the same thing.
    #[test]
    fn the_documentation_answers_are_shared_with_help() {
        let about = about_text().join("\n");
        assert!(about.contains(crate::VERSION), "about does not name the build: {about}");
        assert_eq!(help_text(Some("about")).join("\n"), about);
        assert_eq!(help_text(Some("quickstart")).join("\n"), quickstart_text().join("\n"));

        let issue = help_text(Some("reportissue")).join("\n");
        assert!(issue.contains(ISSUES_URL), "reportissue does not name the tracker: {issue}");
    }
}

#[cfg(test)]
mod capture_phase_tests {
    use super::*;

    /// The capture phase's one word: `snapshot` is real now, not a
    /// placeholder.
    #[test]
    fn the_capture_word_parses_and_is_not_planned() {
        assert_eq!(parse("snapshot"), Some(Ok(Verb::Snapshot)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "snapshot"),
            "`snapshot` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod conversion_phase_tests {
    use super::*;

    /// The conversion phase's first two words: `toimage` and `tohtml` parse
    /// (bare, asking for a destination), and neither is a placeholder.
    #[test]
    fn the_conversion_words_parse_and_are_not_planned() {
        assert_eq!(parse("toimage"), Some(Ok(Verb::ToImage { dest: None })));
        assert_eq!(
            parse("toimage /tmp/out"),
            Some(Ok(Verb::ToImage { dest: Some("/tmp/out".into()) }))
        );
        assert_eq!(parse("tohtml"), Some(Ok(Verb::ToHtml { dest: None })));
        assert_eq!(
            parse("tohtml book.html"),
            Some(Ok(Verb::ToHtml { dest: Some("book.html".into()) }))
        );
        for word in ["toimage", "tohtml"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod split_and_wordcount_tests {
    use super::*;

    /// Split's two shapes — a folder alone, or a count and a folder — and
    /// the one refusal: a chunk of no pages.
    #[test]
    fn split_parses_its_two_shapes() {
        assert_eq!(parse("split"), Some(Ok(Verb::Split { every: 1, dest: None })));
        assert_eq!(
            parse("split /tmp/out"),
            Some(Ok(Verb::Split { every: 1, dest: Some("/tmp/out".into()) }))
        );
        assert_eq!(
            parse("split 3 /tmp/out"),
            Some(Ok(Verb::Split { every: 3, dest: Some("/tmp/out".into()) }))
        );
        assert_eq!(
            parse("split 0 /tmp/out"),
            Some(Err("split: pages per file must be at least 1".into()))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "split"),
            "`split` is still in the planned table"
        );
    }

    #[test]
    fn word_count_parses_and_is_not_planned() {
        assert_eq!(parse("wordcount"), Some(Ok(Verb::WordCount { pages: String::new() })));
        assert_eq!(
            parse("wordcount 2-4"),
            Some(Ok(Verb::WordCount { pages: "2-4".into() }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "wordcount"),
            "`wordcount` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod blank_document_tests {
    use super::*;

    /// Blank paper reads an optional size and an optional ruling, in either
    /// order; anything else is refused with the words that do work.
    #[test]
    fn blankdoc_reads_sizes_and_rulings_in_any_order() {
        let letter = Verb::BlankDoc { width_pt: 612.0, height_pt: 792.0, ruling: Ruling::None };
        assert_eq!(parse("blankdoc"), Some(Ok(letter.clone())));
        assert_eq!(
            parse("blankdoc a4"),
            Some(Ok(Verb::BlankDoc { width_pt: 595.0, height_pt: 842.0, ruling: Ruling::None }))
        );
        assert_eq!(
            parse("blankdoc grid a3"),
            Some(Ok(Verb::BlankDoc {
                width_pt: 842.0,
                height_pt: 1191.0,
                ruling: Ruling::Grid
            }))
        );
        assert_eq!(
            parse("blankdoc legal plain"),
            Some(Ok(Verb::BlankDoc { width_pt: 612.0, height_pt: 1008.0, ruling: Ruling::None }))
        );
        match parse("blankdoc parchment") {
            Some(Err(why)) => assert!(why.contains("parchment"), "unhelpful refusal: {why}"),
            other => panic!("blankdoc parchment parsed as {other:?}"),
        }
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "blankdoc"),
            "`blankdoc` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod combine_tests {
    use super::*;

    #[test]
    fn combine_parses_bare_and_with_a_folder() {
        assert_eq!(parse("combine"), Some(Ok(Verb::Combine { folder: None })));
        assert_eq!(
            parse("combine /tmp/in"),
            Some(Ok(Verb::Combine { folder: Some("/tmp/in".into()) }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "combine"),
            "`combine` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod viewing_phase_tests {
    use super::*;

    #[test]
    fn autoscroll_parses_and_is_not_planned() {
        assert_eq!(parse("autoscroll"), Some(Ok(Verb::AutoScroll)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "autoscroll"),
            "`autoscroll` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod ruler_tests {
    use super::*;

    #[test]
    fn ruler_parses_and_is_not_planned() {
        assert_eq!(parse("ruler"), Some(Ok(Verb::Ruler)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "ruler"),
            "`ruler` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod accessibility_phase_tests {
    use super::*;

    #[test]
    fn the_accessibility_words_parse_and_are_not_planned() {
        assert_eq!(parse("accesscheck"), Some(Ok(Verb::AccessCheck)));
        assert_eq!(parse("accessreport"), Some(Ok(Verb::AccessReport { dest: None })));
        assert_eq!(
            parse("accessreport r.html"),
            Some(Ok(Verb::AccessReport { dest: Some("r.html".into()) }))
        );
        for word in ["accesscheck", "accessreport"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod navigation_phase_tests {
    use super::*;

    #[test]
    fn autobookmarks_parses_and_is_not_planned() {
        assert_eq!(parse("autobookmarks"), Some(Ok(Verb::AutoBookmarks)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "autobookmarks"),
            "`autobookmarks` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    #[test]
    fn preflight_parses_and_is_not_planned() {
        assert_eq!(parse("preflight"), Some(Ok(Verb::Preflight)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "preflight"),
            "`preflight` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod export_images_tests {
    use super::*;

    #[test]
    fn exportimages_parses_and_is_not_planned() {
        assert_eq!(parse("exportimages"), Some(Ok(Verb::ExportAllImages { dest: None })));
        assert_eq!(
            parse("exportimages /tmp/out"),
            Some(Ok(Verb::ExportAllImages { dest: Some("/tmp/out".into()) }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "exportimages"),
            "`exportimages` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod from_files_tests {
    use super::*;

    #[test]
    fn fromfiles_parses_and_is_not_planned() {
        assert_eq!(parse("fromfiles"), Some(Ok(Verb::FromFiles { folder: None })));
        assert_eq!(
            parse("fromfiles /tmp/pics"),
            Some(Ok(Verb::FromFiles { folder: Some("/tmp/pics".into()) }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "fromfiles"),
            "`fromfiles` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod view_setting_tests {
    use super::*;

    #[test]
    fn viewsetting_parses_and_is_not_planned() {
        assert_eq!(parse("viewsetting"), Some(Ok(Verb::ViewSetting)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "viewsetting"),
            "`viewsetting` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod calculator_tests {
    use super::*;

    #[test]
    fn calculator_parses_and_is_not_planned() {
        assert_eq!(parse("calculator"), Some(Ok(Verb::Calculator)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "calculator"),
            "`calculator` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod manage_comments_tests {
    use super::*;

    #[test]
    fn managecomments_parses_and_is_not_planned() {
        assert_eq!(parse("managecomments"), Some(Ok(Verb::ManageComments)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "managecomments"),
            "`managecomments` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod search_highlight_tests {
    use super::*;

    #[test]
    fn searchhighlight_parses_and_is_not_planned() {
        assert_eq!(
            parse("searchhighlight invoice"),
            Some(Ok(Verb::SearchHighlight { words: "invoice".into() }))
        );
        assert_eq!(
            parse("searchhighlight"),
            Some(Ok(Verb::SearchHighlight { words: String::new() }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "searchhighlight"),
            "`searchhighlight` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod compare_tests {
    use super::*;

    #[test]
    fn compare_parses_and_is_not_planned() {
        assert_eq!(parse("compare"), Some(Ok(Verb::Compare { source: None })));
        assert_eq!(
            parse("compare other.pdf"),
            Some(Ok(Verb::Compare { source: Some("other.pdf".into()) }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "compare"),
            "`compare` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod change_color_tests {
    use super::*;

    #[test]
    fn changecolor_parses_every_word_and_refuses_the_rest() {
        assert_eq!(parse("changecolor"), Some(Ok(Verb::ChangeColor(None))));
        assert_eq!(
            parse("changecolor sepia"),
            Some(Ok(Verb::ChangeColor(Some(PaperTint::Sepia))))
        );
        assert_eq!(
            parse("changecolor GRAY"),
            Some(Ok(Verb::ChangeColor(Some(PaperTint::Grey))))
        );
        match parse("changecolor parchment") {
            Some(Err(why)) => assert!(why.contains("parchment"), "unhelpful refusal: {why}"),
            other => panic!("changecolor parchment parsed as {other:?}"),
        }
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "changecolor"),
            "`changecolor` is still in the planned table"
        );
    }

    #[test]
    fn the_bare_form_steps_round_the_ring_and_comes_back() {
        assert_eq!(PaperTint::White.next(), PaperTint::Grey);
        assert_eq!(PaperTint::Grey.next(), PaperTint::Sepia);
        assert_eq!(PaperTint::Sepia.next(), PaperTint::Dark);
        assert_eq!(PaperTint::Dark.next(), PaperTint::White);
        assert_eq!(PaperTint::default(), PaperTint::White);
    }
}

#[cfg(test)]
mod continuous_view_tests {
    use super::*;

    /// The continuous words map onto the layouts this app has always had:
    /// every layout here scrolls through its rows, so "continuous single" is
    /// `Single` and "continuous facing" is `Facing`.
    #[test]
    fn the_continuous_words_map_onto_the_continuous_layouts() {
        assert_eq!(
            parse("viewcontinuous"),
            Some(Ok(Verb::SetLayout(crate::reader::Layout::Single)))
        );
        assert_eq!(
            parse("viewcontinuousfacing"),
            Some(Ok(Verb::SetLayout(crate::reader::Layout::Facing)))
        );
        for word in ["viewcontinuous", "viewcontinuousfacing"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod area_highlight_tests {
    use super::*;

    #[test]
    fn areahighlight_parses_and_is_not_planned() {
        assert_eq!(parse("areahighlight"), Some(Ok(Verb::AreaHighlight)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "areahighlight"),
            "`areahighlight` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    /// Four ribbon buttons whose behaviour this app already had under
    /// another word: the Alias is the same verb, not a stub.
    #[test]
    fn the_alias_words_parse_to_the_behaviour_they_name() {
        assert_eq!(parse("tickmark"), Some(Ok(Verb::FillSign(Some("tick".into())))));
        assert_eq!(parse("inserttext Hello"), Some(Ok(Verb::AddText("Hello".into()))));
        assert_eq!(parse("replacetext"), Some(Ok(Verb::EditText)));
        assert_eq!(parse("addlink"), Some(Ok(Verb::Weblinks)));
        for word in ["tickmark", "inserttext", "replacetext", "addlink"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod picture_alias_tests {
    use super::*;

    /// Custom Stamp and Image Annotation run the picture placement the app
    /// has: a path, or bare asking for the picker, on `Verb::AddImage`.
    #[test]
    fn the_picture_words_run_the_picture_placement() {
        for word in ["customstamp", "imageannotation"] {
            assert_eq!(parse(word), Some(Ok(Verb::AddImage(None))), "{word} bare");
            assert_eq!(
                parse(&format!("{word} logo.png")),
                Some(Ok(Verb::AddImage(Some("logo.png".into())))),
                "{word} with a path"
            );
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod page_marks_tests {
    use super::*;

    #[test]
    fn pagemarks_parses_positions_and_refuses_the_unmeasurable_corners() {
        assert_eq!(
            parse("pagemarks Page {n}"),
            Some(Ok(Verb::PageMarks { top: false, text: "Page {n}".into() }))
        );
        assert_eq!(
            parse("pagemarks topleft Draft {n}"),
            Some(Ok(Verb::PageMarks { top: true, text: "Draft {n}".into() }))
        );
        match parse("pagemarks topright x") {
            Some(Err(why)) => assert!(why.contains("topleft"), "unhelpful refusal: {why}"),
            other => panic!("topright parsed as {other:?}"),
        }
        match parse("pagemarks") {
            Some(Err(why)) => assert!(why.contains("words to write"), "unhelpful refusal: {why}"),
            other => panic!("bare pagemarks parsed as {other:?}"),
        }
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "pagemarks"),
            "`pagemarks` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod from_clipboard_tests {
    use super::*;

    #[test]
    fn fromclipboard_parses_and_is_not_planned() {
        assert_eq!(parse("fromclipboard"), Some(Ok(Verb::FromClipboard)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "fromclipboard"),
            "`fromclipboard` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod pencil_tests {
    use super::*;

    #[test]
    fn pencil_parses_and_is_not_planned() {
        assert_eq!(parse("pencil"), Some(Ok(Verb::Pencil)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "pencil"),
            "`pencil` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod rearrange_tests {
    use super::*;

    #[test]
    fn rearrange_parses_to_the_organize_grid_and_is_not_planned() {
        assert_eq!(parse("rearrange"), Some(Ok(Verb::Thumbnails)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "rearrange"),
            "`rearrange` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod textbox_tests {
    use super::*;

    #[test]
    fn textbox_runs_the_drag_a_box_and_type_flow() {
        assert_eq!(parse("textbox"), Some(Ok(Verb::AddText(String::new()))));
        assert_eq!(parse("textbox Hello"), Some(Ok(Verb::AddText("Hello".into()))));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "textbox"),
            "`textbox` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod from_template_tests {
    use super::*;

    #[test]
    fn fromtemplate_parses_and_is_not_planned() {
        assert_eq!(parse("fromtemplate"), Some(Ok(Verb::FromTemplate { source: None })));
        assert_eq!(
            parse("fromtemplate book.pdf"),
            Some(Ok(Verb::FromTemplate { source: Some("book.pdf".into()) }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "fromtemplate"),
            "`fromtemplate` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod shape_and_keep_tool_tests {
    use super::*;

    #[test]
    fn the_shape_buttons_and_keeptool_parse_and_are_not_planned() {
        assert_eq!(parse("addshape"), Some(Ok(Verb::ShapeWords)));
        assert_eq!(parse("drawing"), Some(Ok(Verb::ShapeWords)));
        assert_eq!(parse("keeptool"), Some(Ok(Verb::KeepTool)));
        for word in ["addshape", "drawing", "keeptool"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
        // The answers must name routes that exist.
        for word in ["line", "rectangle", "circle", "arrow", "pline", "spline", "pencil"] {
            assert!(SHAPE_WORDS.contains(word), "the shape answer omits `{word}`");
        }
    }
}

#[cfg(test)]
mod form_data_tests {
    use super::*;

    #[test]
    fn formexport_and_formimport_parse_and_are_not_planned() {
        assert_eq!(parse("formexport"), Some(Ok(Verb::FormExport { dest: None })));
        assert_eq!(
            parse("formexport data.json"),
            Some(Ok(Verb::FormExport { dest: Some("data.json".into()) }))
        );
        assert_eq!(parse("formimport"), Some(Ok(Verb::FormImport { dest: None })));
        for word in ["formexport", "formimport"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod form_tab_tests {
    use super::*;

    #[test]
    fn the_form_tab_words_parse_and_are_not_planned() {
        assert_eq!(parse("formtosheet"), Some(Ok(Verb::FormToSheet { dest: None })));
        assert_eq!(parse("resetform"), Some(Ok(Verb::ResetForm)));
        assert_eq!(
            parse("tooltip email Your address"),
            Some(Ok(Verb::ToolTip { field: "email".into(), text: "Your address".into() }))
        );
        match parse("tooltip email") {
            Some(Err(why)) => assert!(why.contains("field and its words"), "{why}"),
            other => panic!("bare tooltip parsed as {other:?}"),
        }
        for word in ["formtosheet", "tooltip", "resetform"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod attach_flatten_tests {
    use super::*;

    #[test]
    fn flatten_attach_and_attachcomment_parse_and_are_not_planned() {
        assert_eq!(parse("flatten"), Some(Ok(Verb::Flatten)));
        assert_eq!(parse("attach"), Some(Ok(Verb::Attach { source: None })));
        assert_eq!(
            parse("attach notes.txt"),
            Some(Ok(Verb::Attach { source: Some("notes.txt".into()) }))
        );
        assert_eq!(
            parse("attachcomment letter.pdf"),
            Some(Ok(Verb::AttachComment { source: Some("letter.pdf".into()) }))
        );
        for word in ["flatten", "attach", "attachcomment"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod tags_panel_tests {
    use super::*;

    #[test]
    fn tagspanel_parses_and_is_not_planned() {
        assert_eq!(parse("tagspanel"), Some(Ok(Verb::TagSpanel)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "tagspanel"),
            "`tagspanel` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod alt_text_tests {
    use super::*;

    #[test]
    fn alttext_parses_and_is_not_planned() {
        assert_eq!(
            parse("alttext"),
            Some(Ok(Verb::AltText { pages: String::new() }))
        );
        assert_eq!(
            parse("alttext 1-3"),
            Some(Ok(Verb::AltText { pages: "1-3".into() }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "alttext"),
            "`alttext` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod reading_order_tests {
    use super::*;

    #[test]
    fn readingorder_parses_and_is_not_planned() {
        assert_eq!(
            parse("readingorder"),
            Some(Ok(Verb::ReadingOrder { pages: String::new() }))
        );
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "readingorder"),
            "`readingorder` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod view_split_tests {
    use super::*;

    #[test]
    fn viewsplit_parses_and_is_not_planned() {
        assert_eq!(parse("viewsplit"), Some(Ok(Verb::ViewSplit)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "viewsplit"),
            "`viewsplit` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod reading_options_tests {
    use super::*;

    #[test]
    fn readingoptions_parses_and_is_not_planned() {
        assert_eq!(parse("readingoptions"), Some(Ok(Verb::ReadingOptions)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "readingoptions"),
            "`readingoptions` is still in the planned table"
        );
        for route in ["viewsetting", "changecolor", "Read Out Loud"] {
            assert!(READING_OPTIONS_WORDS.contains(route), "the answer omits `{route}`");
        }
    }
}

#[cfg(test)]
mod reverse_view_tests {
    use super::*;

    #[test]
    fn reverseview_parses_and_is_not_planned() {
        assert_eq!(parse("reverseview"), Some(Ok(Verb::ReverseView)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "reverseview"),
            "`reverseview` is still in the planned table"
        );
    }
}

#[cfg(test)]
mod stamp_callout_tests {
    use super::*;

    #[test]
    fn stamp_and_callout_parse_and_are_not_planned() {
        assert_eq!(parse("stamp Approved"), Some(Ok(Verb::Stamp { words: "Approved".into() })));
        assert_eq!(
            parse("callout see figure 2"),
            Some(Ok(Verb::Callout { words: "see figure 2".into() }))
        );
        match parse("stamp") {
            Some(Err(why)) => assert!(why.contains("words to stamp"), "{why}"),
            other => panic!("bare stamp parsed as {other:?}"),
        }
        for word in ["stamp", "callout"] {
            assert!(
                PLANNED.iter().all(|(v, _, _)| *v != word),
                "`{word}` is still in the planned table"
            );
        }
    }
}

#[cfg(test)]
mod mark_redact_tests {
    use super::*;

    #[test]
    fn markredact_parses_and_is_not_planned() {
        assert_eq!(parse("markredact"), Some(Ok(Verb::MarkRedact)));
        assert!(
            PLANNED.iter().all(|(v, _, _)| *v != "markredact"),
            "`markredact` is still in the planned table"
        );
    }
}
