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
    /// somebody clicks — the mark a person writes on a form. It says nothing
    /// about who drew it, and the tool that does is `certify`. Anything that
    /// blurs the two is worse than no signature at all.
    ///
    /// `draw` makes a new one instead of placing the last.
    Signature { draw: bool },
    /// Check the signatures this document carries.
    ///
    /// Says whether the file has changed since it was signed. **Not** whether
    /// the signer is who they say — that needs a chain of trust this program
    /// does not have, and every answer says so.
    Validate,
    /// Ask a time authority to attest that this document exists now.
    ///
    /// **The only verb that uses the network.** It takes the authority's
    /// address because there is no sensible default — and because contacting
    /// somewhere nobody named is not a thing this program should do. A digest
    /// leaves the machine; the document does not.
    TimeStamp(Option<String>),
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

    // Review — phase 10.
    /// Find text across the document.
    Find(String),
    /// Step to the next or previous match.
    FindStep { forward: bool },
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
    /// Say what kind of text this page has, and what that means.
    TextLayer,
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
    Override {
        token: "measure",
        kernel_meaning: "draw points at intervals along an object",
        why: "There is nothing to divide by length in a PDF, and a reader's \
              `measure` is the page's own: `measure distance` / `measure area`, \
              after `calibrate` fixes the page's scale. The kernel's MEASURE \
              needs geometry to walk along, which only exists while the Draw \
              rail is in use — and it keeps its own alias for exactly that.",
        kernel_keeps: Some("me"),
    },
    Override {
        token: "find",
        kernel_meaning: "find text in the drawing",
        why: "The document here is a PDF, so the word searches its text layer — \
              what a reader means by `find`, and what the Search & Replace tool \
              is built on. The kernel's find, like its replace, only has \
              geometry to walk when the drawing namespace is in use.",
        kernel_keeps: Some("findtext"),
    },
    Override {
        token: "replace",
        kernel_meaning: "find and replace text in the drawing",
        why: "Recognised and planned: it names the Search & Replace tool of the \
              editing phase and says so rather than answering `unknown \
              command`. When built it rewrites the PDF's own text layer through \
              pdf_core — never the kernel's drawing text.",
        kernel_keeps: Some("findreplace"),
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
    ("about", "the documentation phase", "the About Pagify tool"),
    ("accesscheck", "the accessibility phase", "the Full Check tool"),
    ("accessreport", "the accessibility phase", "the Accessibility Report tool"),
    ("add3d", "the editing phase", "the Add 3D tool"),
    ("addimage", "the editing phase", "the Add Images tool"),
    ("addlink", "the editing phase", "the Link tool"),
    ("addshape", "the editing phase", "the Add Shapes tool"),
    ("alttext", "the accessibility phase", "the Set Alternate Text tool"),
    ("areahighlight", "the annotation phase", "the Area Highlight tool"),
    ("articlebox", "the editing phase", "the Add Article Box tool"),
    ("assistant", "the viewing phase", "the Assistant tool"),
    ("attach", "the editing phase", "the File Attachment tool"),
    ("attachcomment", "the annotation phase", "the File tool"),
    ("autobookmarks", "the navigation phase", "the Auto Create Bookmarks tool"),
    ("autoscroll", "the viewing phase", "the AutoScroll tool"),
    ("autotag", "the accessibility phase", "the Autotag Document tool"),
    ("blankdoc", "the conversion phase", "the Blank tool"),
    ("bookmark", "the navigation phase", "the Bookmark tool"),
    ("calcorder", "the forms phase", "the Calculation Order tool"),
    ("calculator", "the annotation phase", "the Accounting Calculator tool"),
    ("callout", "the annotation phase", "the Callout tool"),
    ("changecolor", "the viewing phase", "the Change Color tool"),
    ("checkupdates", "the documentation phase", "the Check for Updates tool"),
    ("clipboard", "the capture phase", "the Clipboard tool"),
    ("cloudstorage", "the sharing phase", "the Cloud Storage tool"),
    ("combfield", "the forms phase", "the Comb Field tool"),
    ("combine", "the conversion phase", "the Combine Files tool"),
    ("compare", "the viewing phase", "the Compare tool"),
    ("createform", "the forms phase", "the Form tool"),
    ("crossref", "the editing phase", "the Cross Reference tool"),
    ("customstamp", "the annotation phase", "the Custom Stamp tool"),
    ("drawing", "the annotation phase", "the Drawing tool"),
    ("editxfa", "the forms phase", "the Edit Static XFA Form tool"),
    ("email", "the sharing phase", "the Email tool"),
    ("emailattach", "the sharing phase", "the Attach to Email tool"),
    ("exportimages", "the conversion phase", "the Export All Images tool"),
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
    ("flatten", "the page phase", "the Flatten tool"),
    ("formdesigner", "the forms phase", "the Designer Assistant tool"),
    ("formexport", "the forms phase", "the Export tool"),
    ("formimport", "the forms phase", "the Import tool"),
    ("formrecognise", "the forms phase", "the Run Form Field Recognition tool"),
    ("formtosheet", "the forms phase", "the Form to sheet tool"),
    ("fromclipboard", "the conversion phase", "the From Clipboard tool"),
    ("fromfiles", "the conversion phase", "the From Files tool"),
    ("fromscanner", "the scanning phase", "the From Scanner tool"),
    ("fromtemplate", "the conversion phase", "the From Template tool"),
    ("imageannotation", "the annotation phase", "the Image Annotation tool"),
    ("inserttext", "the annotation phase", "the Insert Text tool"),
    ("interleave", "the page phase", "the Interleaving tool"),
    ("javascript", "the forms phase", "the JavaScript tool"),
    ("jointext", "the editing phase", "the Link & Join Text tool"),
    ("keeptool", "the annotation phase", "the Keep Tool Selected tool"),
    ("managecomments", "the annotation phase", "the Manage Comments tool"),
    ("markredact", "the protection phase", "the Mark for Redaction tool"),
    ("media", "the editing phase", "the Audio & Video tool"),
    ("onlineform", "the signing phase", "the Create Online Form tool"),
    ("pagemarks", "the page phase", "the Page Marks tool"),
    ("pagetemplates", "the forms phase", "the Page Templates tool"),
    ("pencil", "the annotation phase", "the Pencil tool"),
    ("portfolio", "the conversion phase", "the PDF Portfolio tool"),
    ("preflight", "the conversion phase", "the Preflight tool"),
    ("quickstart", "the documentation phase", "the Quick Start tool"),
    ("readaloud", "the viewing phase", "the Read tool"),
    ("readingoptions", "the accessibility phase", "the Reading Options tool"),
    ("readingorder", "the accessibility phase", "the Reading Order tool"),
    ("rearrange", "the page phase", "the Rearrange tool"),
    ("replace", "the editing phase", "the Search & Replace tool"),
    ("replacepage", "the page phase", "the Replace tool"),
    ("replacetext", "the annotation phase", "the Replace Text tool"),
    ("reportissue", "the documentation phase", "the Report an Issue tool"),
    ("requestsignature", "the signing phase", "the Request Signature tool"),
    ("resetform", "the forms phase", "the Reset Form tool"),
    ("reverseview", "the viewing phase", "the Reverse View tool"),
    ("ruler", "the viewing phase", "the Toggle Ruler tool"),
    ("searchhighlight", "the annotation phase", "the Search & Highlight tool"),
    ("sendbulk", "the signing phase", "the Send in Bulk tool"),
    ("sendreview", "the sharing phase", "the Send for Review tool"),
    ("sharelink", "the sharing phase", "the Share Link tool"),
    ("shortcuts", "the documentation phase", "the Keyboard Shortcuts tool"),
    ("signbranding", "the signing phase", "the Add E-Sign Branding tool"),
    ("snapshot", "the capture phase", "the Snapshot tool"),
    ("spelling", "the editing phase", "the Check Spelling tool"),
    ("split", "the page phase", "the Split tool"),
    ("stamp", "the annotation phase", "the Stamp tool"),
    ("tagspanel", "the accessibility phase", "the Tags Panel tool"),
    ("textbox", "the annotation phase", "the Textbox tool"),
    ("thumbnails", "the page phase", "the Thumbnail View tool"),
    ("tickmark", "the annotation phase", "the TickMark tool"),
    ("tohtml", "the conversion phase", "the To HTML tool"),
    ("toimage", "the conversion phase", "the To Image tool"),
    ("tooffice", "the conversion phase", "the To MS Office tool"),
    ("toolsettings", "the forms phase", "the Tool Settings tool"),
    ("tooltip", "the forms phase", "the Add Tooltip tool"),
    ("toother", "the conversion phase", "the To Other tool"),
    ("trackreviews", "the sharing phase", "the Track Reviews tool"),
    ("transitions", "the viewing phase", "the Page Transitions tool"),
    ("viewcontinuous", "the viewing phase", "the Continuous tool"),
    ("viewcontinuousfacing", "the viewing phase", "the Continuous Facing tool"),
    ("viewsetting", "the viewing phase", "the View Setting tool"),
    ("viewsplit", "the viewing phase", "the Split tool"),
    ("weblinks", "the editing phase", "the Web Links tool"),
    ("wordcount", "the viewing phase", "the Word Count tool"),
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

    let parsed = match head.as_str() {
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
        "secure" => SecureOptions::parse(tail).map(Verb::Secure),
        "whiteout" => Ok(Verb::Whiteout),
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
        "documentstatus" | "status" => Ok(Verb::DocumentStatus),
        "applysignatures" => Ok(Verb::ApplySignatures),
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
        "signature" => match tail.trim().to_ascii_lowercase().as_str() {
            "" | "place" | "put" | "sign" => Ok(Verb::Signature { draw: false }),
            "draw" | "new" | "create" => Ok(Verb::Signature { draw: true }),
            other => Err(format!(
                "signature: don't know {other:?} — `signature` places the one you \
                 drew, `signature draw` makes a new one"
            )),
        },
        "validate" => Ok(Verb::Validate),
        "timestamp" => {
            let tail = tail.trim();
            if tail.is_empty() {
                Ok(Verb::TimeStamp(None))
            } else {
                Ok(Verb::TimeStamp(Some(tail.to_string())))
            }
        }
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
        "copy" | "copytext" => Ok(Verb::CopyText),
        "reversepages" => Ok(Verb::ReversePages),
        // An inset rather than a rectangle. Trimming the same amount off every
        // edge is what "crop the margins" means, and a rectangle typed into a
        // box is four numbers nobody can picture.
        "edittext" => Ok(Verb::EditText),
        "addtext" | "typewriter" => Ok(Verb::AddText(tail.to_string())),
        "viewsingle" => Ok(Verb::SetLayout(crate::reader::Layout::Single)),
        "viewfacing" => Ok(Verb::SetLayout(crate::reader::Layout::Facing)),
        "viewcover" => Ok(Verb::SetLayout(crate::reader::Layout::FacingWithCover)),
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
                Err("usage: saveas <path.pdf>".into())
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
        "textlayer" | "whytext" => Ok(Verb::TextLayer),
        "quit" | "exit" => Ok(Verb::Quit { force: false }),
        "quit!" | "exit!" => Ok(Verb::Quit { force: true }),
        "help" => Ok(Verb::Help(rest.first().map(|s| s.to_string()))),

        _ => return None,
    };

    Some(parsed)
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
        "rotate", "undo", "redo", "pdfium", "quit", "exit", "help",
        "close!", "quit!", "exit!",
        // Several page operations are spelt out rather than taking the short
        // word: `delete`, `insert`, `scale`, `mp` and `dist` are all live
        // drawing commands in the kernel, and a PDF verb that shadowed them
        // would take away erase, block insert, the scale tool, match-properties
        // and distance from anyone who also uses SIMLUX. The guard test in
        // tests/command_box.rs is what caught every one of them.
        "extract", "import", "deletepage", "delpage", "insertpage", "movepage",
        "highlight", "hl", "note", "calibrate", "cal", "pagescale", "measure",
        "find", "findnext", "fn", "findprev", "fp", "reflow",
        "record", "stop", "endrecord", "replay", "pick", "textlayer", "whytext",
        "outlinedfont", "lock", "lockall", "lockarea", "unlock", "secure",
        "hiddendata", "sanitize", "sanitise", "smartredact", "whiteout", "sensitivity", "fillsign", "certify", "timestamp", "validate", "signature", "managesignatures", "applysignatures", "documentstatus", "status", "signrectangle", "signline", "signcheck", "signcross", "signdot", "predefinedtext", "moveobject", "editobject",
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
        "planned    redact sign comment".into(),
        "drawing    every SIMLUX command — line, polyline, trim, fillet, offset, … — with the same aliases".into(),
        "           `help <verb>` explains any word that means something different here.".into(),
        "".into(),
        "Page operations are spelt out — `deletepage`, `insertpage`, `pagescale` — because".into(),
        "`delete`, `insert` and `scale` are drawing commands and keep their SIMLUX meanings.".into(),
    ]
}
