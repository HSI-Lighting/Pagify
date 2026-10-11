//! What a command *does*, as data — the design review's Phase 4b
//! (DESIGN_REVIEW.md §3.4), complete.
//!
//! [`plan`] turns every document, view and app verb into [`Effect`]s that
//! the app applies; its router is exhaustive, so a new verb must choose a
//! domain before the crate compiles. `Help` and `Planned` are the only verbs
//! with no plan — the app's `run()` answers them before dispatch reaches
//! `act`.
//!
//! Planning is deliberately pure: no engine, no window, no `egui`. That is
//! what makes the decision side testable in this crate, while the app keeps
//! ownership of *doing* things (`Effect` application lives in
//! `pagify_app::dispatch`).

use crate::page_space::{AppPoint, PageSpace};
use crate::verbs::{MeasureKind, PageTarget, SignatureAction, Verb, ZoomTarget};
use cad_kernel::Vec2;
use pdf_core::document::Stacking;

/// What the plan may need to know about the app at the moment the verb runs.
///
/// Kept small on purpose: every field here is a decision the plan makes
/// instead of the old handlers (document guards, the current page, the page
/// height the kernel-to-app flip needs, whether a pick has somewhere to go).
#[derive(Debug, Clone, Copy)]
pub struct CommandEnv {
    /// A document is open in the active tab.
    pub has_doc: bool,
    /// The active tab's current page.
    pub page: usize,
    /// That page's height in points, for the kernel/app flip; 792 when
    /// unknown, the same fallback the app used.
    pub page_height: f64,
    /// A tool is armed and waiting for a click.
    pub tool_armed: bool,
    /// How many signatures the library holds (drives the names a new pad
    /// gets, and whether placing needs one drawn first).
    pub signature_count: usize,
}

impl Default for CommandEnv {
    fn default() -> Self {
        Self { has_doc: false, page: 0, page_height: 792.0, tool_armed: false, signature_count: 0 }
    }
}

/// A user-visible consequence of running a command. The app applies these in
/// order; nothing here knows how.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Open a path in its own tab.
    OpenPath(std::path::PathBuf),
    /// Ask the platform for a file to open.
    OpenFileDialog,
    /// Close the current document, asking first when `force` is false and
    /// the tab has unsaved work.
    CloseDocument { force: bool },
    /// Leave the program, asking first when `force` is false and any tab has
    /// unsaved work.
    Quit { force: bool },
    /// Go to a page: a number, `First`, `Last` or `Next`.
    GoTo(PageTarget),
    /// Set the zoom: `Fit`, `Width` or a percentage.
    Zoom(ZoomTarget),
    /// Rotate the view by degrees.
    RotatePage(i32),
    /// Report whether the page has a text layer the editor can use.
    ReportTextLayer,
    /// Copy the page in view to the system clipboard as an image. The render
    /// needs no window; the clipboard write does, so the app carries it out
    /// in the frame's own clipboard pass.
    Snapshot,
    /// Turn the view's own scrolling on or off.
    ToggleAutoScroll,
    /// Turn the page ruler on or off.
    ToggleRuler,
    /// Run the accessibility checks this build can make.
    RunAccessCheck,
    /// Write those findings as one HTML file; `None` asks for the file
    /// first.
    WriteAccessReport { dest: Option<std::path::PathBuf> },
    /// Make a bookmark per page from the pages' own largest text runs, as
    /// one batch.
    RunAutoBookmarks,
    /// Run the document checks this build can make.
    RunPreflight,
    /// Write every embedded picture out as a PNG; `None` asks for the folder
    /// first.
    ExportPictures { dest: Option<std::path::PathBuf> },
    /// Make a new document out of nothing, one sheet of this size and ruling.
    BlankDocument { width_pt: f32, height_pt: f32, ruling: pdf_core::document::Ruling },
    /// Merge PDFs into one new document; a folder to read them from, or
    /// `None` to ask with a picker.
    CombineFiles { folder: Option<std::path::PathBuf> },
    /// Say a line in the info colour.
    Say(String),
    /// Start an update check; the caller has already said it is checking.
    CheckUpdate,
    /// Toggle the floating Layers panel, with its explanation on open and
    /// "closed" on close.
    ToggleLayers,
    /// Repair the document's locks, saying what was repaired.
    RepairLocks,
    /// Set the opacity of the picked object; `percent` is 0–100.
    SetOpacity(f32),
    /// Move the picked object to the front or the back.
    Restack(Stacking),
    /// Arm the area tool that locks whatever it covers.
    ArmLockArea,
    /// Ask for a passcode covering the pages `spec` names.
    LockPages(String),
    /// Ask for a passcode to lift every lock in the document.
    Unlock,
    /// Arm two-point calibration at a known real-world distance.
    ArmCalibrate { distance: f64, unit: String },
    /// Report the current measure scale.
    ReportScale,
    /// Arm the distance or area measure tool.
    ArmMeasure(MeasureKind),
    /// Start recording a script under this name.
    Record(String),
    /// Stop recording and write the script out.
    StopRecording,
    /// Replay a recorded script.
    Replay(std::path::PathBuf),
    /// Say a line in the error colour.
    SayError(String),
    /// Deliver a typed pick, already flipped from kernel space to app space.
    TakePick(AppPoint),
    /// Run the `sensitivity` command's reporting/marking/taking-off.
    Sensitivity(Option<String>),
    /// Run the `fillsign` family of form marks.
    FillSign(Option<String>),
    /// Open the predefined-text list, or use the given words.
    PredefinedText(Option<String>),
    /// Take up the object tool: editing when `edit`, moving otherwise.
    TakeUpObjectTool { edit: bool, page: usize },
    /// Arm the drawn-line form mark.
    ArmSignLine { page: usize },
    /// Arm the drawn-rectangle form mark.
    ArmSignRectangle { page: usize },
    /// Say the document-status lines.
    ReportDocumentStatus,
    /// Run the saved-signature confirmation pass over every page.
    RunApplySignatures,
    /// Run the signature-management actions.
    RunManageSignatures(crate::verbs::Signatures),
    /// Report where the session log is, if it could be written.
    ReportSessionLog,
    /// Say the window's key bindings. The bindings live in the app (`Keys` in
    /// `main.rs`), which is the only place they are real.
    ReportShortcuts,
    /// Open the signature pad with this name; `then_place` carries on to
    /// placing once it is drawn.
    OpenSignaturePad { name: String, then_place: bool },
    /// Add a signature from this picture file.
    UploadSignaturePath(std::path::PathBuf),
    /// Ask the platform for a picture to add as a signature.
    UploadSignatureDialog,
    /// Arm placing the library's current signature.
    ArmSignature { page: usize },
    /// Run the signature-validation report.
    RunValidate,
    /// Run `certify`: write the certification at this path, or report when
    /// `None`.
    RunCertify(Option<std::path::PathBuf>),
    /// Arm the whiteout tool.
    ArmWhiteout,
    /// Arm the arrow drawing tool.
    ArmDrawArrow,
    /// Toggle whether new rectangles and circles are drawn filled.
    ToggleFill,
    /// Arm the redaction tool.
    ArmRedact,
    /// Lock the text selection, or say to make one first.
    LockWords,
    /// Run `secure` with these options — the document's password state
    /// decides the prompt.
    RunSecure(crate::verbs::SecureOptions),
    /// Run the Smart Redact pass.
    RunSmartRedact { redact: bool },
    /// Survey or clean the document's hidden data.
    RunHiddenData { clean: bool },
    /// Take the document's password off.
    RunUnsecure,
    /// Finish the armed pick if it can be finished, else say why not.
    FinishPick,
    /// Save the document (over itself when possible).
    SaveFile,
    /// Save the document to this path.
    SaveFileAs(std::path::PathBuf),
    /// Ask the platform where to save.
    SaveAsDialog,
    /// Extract pages to a file.
    RunExtract { pages: String, dest: std::path::PathBuf },
    /// Import pages from a file.
    RunImport { source: std::path::PathBuf, pages: String },
    /// Delete pages.
    RunDeletePages(String),
    /// Insert a blank page.
    RunInsertPage,
    /// Move pages before a page.
    RunMovePages { pages: String, before: usize },
    /// Replace the page in view with pages from a file; `None` asks for the
    /// file first.
    ReplacePage { source: Option<std::path::PathBuf>, pages: String },
    /// Alternate a file's pages into this document after the page in view;
    /// `None` asks for the file first.
    InterleavePages { source: Option<std::path::PathBuf>, pages: String },
    /// Write every page as a PNG into this folder; `None` asks for the
    /// folder first.
    ExportImages { dest: Option<std::path::PathBuf> },
    /// Write the document's text as one HTML file; `None` asks for the file
    /// first.
    ExportHtml { dest: Option<std::path::PathBuf> },
    /// Write the document as several files, this many pages each; `None`
    /// asks for the folder first.
    SplitPages { every: usize, dest: Option<std::path::PathBuf> },
    /// Report words and characters over these pages — empty means all.
    ReportWordCount(String),
    /// Reflow the page's text.
    RunReflow,
    /// Search for `needle`.
    RunFind(String),
    /// Step to the next or previous hit.
    RunFindStep { forward: bool },
    /// Open the Search & Replace panel.
    OpenFindReplace,
    /// Open the Check Spelling panel.
    OpenSpellCheck,
    /// Bookmark the page in view.
    AddBookmark,
    /// Arm the article-box tool.
    ArmArticleBox,
    /// Begin making a web link out of the selection.
    BeginWebLink,
    /// Begin joining runs into one paragraph.
    BeginJoinText,
    /// Begin sampling properties to match.
    BeginMatchProperties,
    /// Mark the selection.
    RunMarkText(crate::verbs::Markup),
    /// List the page's marks.
    RunListMarks,
    /// Remove the nth mark.
    RunRemoveMark(usize),
    /// Add a note.
    RunNote(String),
    /// Step the undo/redo history; `undo` picks the direction.
    UndoRedo { undo: bool },
    /// Set the pointer tool (select or hand).
    SetPointer(crate::verbs::PointerMode),
    /// Copy with the frame's own precedence (page, object, then text).
    CopyText,
    /// Paste with the frame's own precedence (page, then object).
    Paste,
    /// Open the run editor on the words under the pointer.
    EditText,
    /// Add the given words, or open the type-first flow when empty.
    AddText(String),
    /// Add a picture from this path, or ask for one.
    AddImage(Option<std::path::PathBuf>),
    /// Set the reader's layout.
    SetLayout(crate::reader::Layout),
    /// Reverse the page order.
    RunReversePages,
    /// Toggle the Organize grid.
    ToggleOrganizeGrid,
    /// Toggle light/dark appearance.
    ToggleAppearance,
    /// Duplicate the named pages.
    RunDuplicatePages(String),
    /// Crop the named pages by a margin.
    RunCropPages { pages: String, margin: f32 },
    /// Resize the named pages.
    RunResizePages { pages: String, width_pt: f32, height_pt: f32 },
    /// Swap two pages.
    RunSwapPages { a: usize, b: usize },
    /// Rotate the named pages by quarters.
    RunRotatePages { pages: String, quarters: i32 },
    /// Extract the named pages' text.
    RunExtractText(String),
    /// Add, remove or clear the outlined-font set.
    RunOutlinedFont(crate::verbs::OutlinedFontAction),
    /// Clear the recent-documents list.
    ClearRecent,
}

/// The plan for a verb — what it does, as data.
///
/// The router below names every [`Verb`] on purpose: it is exhaustive, so a
/// new verb cannot be added without being put in a domain (the compiler
/// refuses the match, where the old wildcard let it through to a runtime
/// panic). `Help` and `Planned` are the only verbs with no plan; the app's
/// `run()` answers them before dispatch. Each domain's arms live in their
/// own function so no single function outgrows the review's size invariant.
pub fn plan(verb: &Verb, env: &CommandEnv) -> Option<Vec<Effect>> {
    match verb {
        Verb::Open(_)
        | Verb::OpenDialog
        | Verb::Close { .. }
        | Verb::Quit { .. }
        | Verb::Page(_)
        | Verb::Zoom(_)
        | Verb::RotatePage(_)
        | Verb::TextLayer
        | Verb::Pdfium
        | Verb::Version
        | Verb::CheckUpdate
        | Verb::Snapshot
        | Verb::AutoScroll
        | Verb::Ruler
        | Verb::AccessCheck
        | Verb::AccessReport { .. }
        | Verb::AutoBookmarks
        | Verb::Preflight
        | Verb::ExportAllImages { .. }
        | Verb::BlankDoc { .. }
        | Verb::Combine { .. } => plan_document(verb, env),
        Verb::Layers
        | Verb::RepairLocks
        | Verb::Opacity(_)
        | Verb::BringToFront
        | Verb::SendToBack
        | Verb::LockArea
        | Verb::LockPages(_)
        | Verb::Unlock
        | Verb::Calibrate { .. }
        | Verb::Scale
        | Verb::Measure(_)
        | Verb::Record(_)
        | Verb::StopRecording
        | Verb::Replay(_) => plan_objects_and_measurement(verb, env),
        Verb::Pick(_)
        | Verb::Sensitivity(_)
        | Verb::FillSign(_)
        | Verb::PredefinedText(_)
        | Verb::EditObject
        | Verb::MoveThing
        | Verb::SignLine
        | Verb::SignRectangle
        | Verb::DocumentStatus => plan_tools(verb, env),
        Verb::SessionLog
        | Verb::ApplySignatures
        | Verb::ManageSignatures(_)
        | Verb::Signature(_) => plan_signing(verb, env),
        Verb::Validate
        | Verb::Certify(_)
        | Verb::Whiteout
        | Verb::DrawArrow
        | Verb::ToggleFill
        | Verb::Redact
        | Verb::Lock
        | Verb::Secure(_)
        | Verb::SmartRedact { .. }
        | Verb::HiddenData { .. }
        | Verb::Unsecure => plan_security(verb, env),
        Verb::Finish
        | Verb::Save
        | Verb::SaveAs(_)
        | Verb::SaveAsDialog
        | Verb::Extract { .. }
        | Verb::Import { .. }
        | Verb::DeletePages(_)
        | Verb::InsertPage
        | Verb::MovePages { .. }
        | Verb::ReplacePage { .. }
        | Verb::Interleave { .. }
        | Verb::ToImage { .. }
        | Verb::ToHtml { .. }
        | Verb::Split { .. }
        | Verb::WordCount { .. }
        | Verb::Reflow
        | Verb::Find(_)
        | Verb::FindStep { .. }
        | Verb::Replace
        | Verb::Spelling
        | Verb::Bookmark
        | Verb::ArticleBox
        | Verb::Weblinks
        | Verb::JoinText
        | Verb::MatchProperties
        | Verb::Copy
        | Verb::MarkText(_)
        | Verb::ListMarks
        | Verb::RemoveMark(_)
        | Verb::Note(_) => plan_files_and_text(verb, env),
        Verb::Undo
        | Verb::Redo
        | Verb::Pointer(_)
        | Verb::CopyText
        | Verb::Paste
        | Verb::EditText
        | Verb::AddText(_)
        | Verb::AddImage(_)
        | Verb::SetLayout(_)
        | Verb::ReversePages
        | Verb::Thumbnails
        | Verb::ToggleAppearance
        | Verb::DuplicatePages(_)
        | Verb::CropPages { .. }
        | Verb::ResizePages { .. }
        | Verb::SwapPages { .. }
        | Verb::RotatePages { .. }
        | Verb::ExtractText(_)
        | Verb::OutlinedFont(_)
        | Verb::ClearHistory => plan_navigation(verb, env),
        Verb::About
        | Verb::Shortcuts
        | Verb::QuickStart
        | Verb::ReportIssue => plan_documentation(verb, env),
        Verb::Help(_) | Verb::Planned { .. } => None,
    }
}

/// The document and view verbs.
fn plan_document(verb: &Verb, env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        // Opening lands in its own tab, so it never puts this one's unsaved
        // work at risk — see the app's `open_with`.
        Verb::Open(path) => vec![Effect::OpenPath(path.clone())],
        Verb::OpenDialog => vec![Effect::OpenFileDialog],
        Verb::Close { force } => vec![Effect::CloseDocument { force: *force }],
        Verb::Quit { force } => vec![Effect::Quit { force: *force }],
        Verb::Page(target) => vec![Effect::GoTo(target.clone())],
        Verb::Zoom(target) => vec![Effect::Zoom(target.clone())],
        Verb::RotatePage(degrees) => vec![Effect::RotatePage(*degrees)],
        Verb::TextLayer => vec![Effect::ReportTextLayer],
        // The one capture verb: a page in view is what there is to copy.
        Verb::Snapshot if !env.has_doc => vec![Effect::SayError("nothing open.".into())],
        Verb::Snapshot => vec![Effect::Snapshot],
        Verb::AutoScroll => vec![Effect::ToggleAutoScroll],
        Verb::Ruler => vec![Effect::ToggleRuler],
        // Nothing open is nothing to check; the app's own report says so.
        Verb::AccessCheck => vec![Effect::RunAccessCheck],
        Verb::AccessReport { dest } => vec![Effect::WriteAccessReport { dest: dest.clone() }],
        Verb::AutoBookmarks => vec![Effect::RunAutoBookmarks],
        Verb::Preflight => vec![Effect::RunPreflight],
        Verb::ExportAllImages { dest } => vec![Effect::ExportPictures { dest: dest.clone() }],
        // A new document is allowed with nothing open — that is the point of
        // it.
        Verb::BlankDoc { width_pt, height_pt, ruling } => vec![Effect::BlankDocument {
            width_pt: *width_pt,
            height_pt: *height_pt,
            ruling: *ruling,
        }],
        // Combining makes its own new document, so it needs nothing open
        // either.
        Verb::Combine { folder } => vec![Effect::CombineFiles { folder: folder.clone() }],
        Verb::Pdfium => vec![Effect::Say(crate::pdfium::describe())],
        Verb::Version => vec![Effect::Say(format!("Pagify {}", crate::VERSION))],
        // The message comes first so it is on the bar while the check runs.
        Verb::CheckUpdate => vec![Effect::Say("checking for a newer build…".into()), Effect::CheckUpdate],
        _ => return None,
    })
}

/// The objects and measurement verbs.
fn plan_objects_and_measurement(verb: &Verb, _env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        // Mirrors of the verbs, so the app applies the same bodies it always
        // ran.
        Verb::Layers => vec![Effect::ToggleLayers],
        Verb::RepairLocks => vec![Effect::RepairLocks],
        Verb::Opacity(percent) => vec![Effect::SetOpacity(*percent)],
        Verb::BringToFront => vec![Effect::Restack(Stacking::Front)],
        Verb::SendToBack => vec![Effect::Restack(Stacking::Back)],
        Verb::LockArea => vec![Effect::ArmLockArea],
        Verb::LockPages(spec) => vec![Effect::LockPages(spec.clone())],
        Verb::Unlock => vec![Effect::Unlock],
        Verb::Calibrate { distance, unit } => {
            vec![Effect::ArmCalibrate { distance: *distance, unit: unit.clone() }]
        }
        Verb::Scale => vec![Effect::ReportScale],
        Verb::Measure(kind) => vec![Effect::ArmMeasure(*kind)],
        Verb::Record(name) => vec![Effect::Record(name.clone())],
        Verb::StopRecording => vec![Effect::StopRecording],
        Verb::Replay(path) => vec![Effect::Replay(path.clone())],
        _ => return None,
    })
}

/// The tool verbs: the guards and the one coordinate flip live here, where
/// they can be tested without a window.
fn plan_tools(verb: &Verb, env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        Verb::Pick(at) => {
            if !env.tool_armed {
                vec![Effect::SayError("nothing is waiting for a click.".into())]
            } else {
                // A typed pick is in the *same* coordinates as a typed draw
                // command — `l 30,250 170,250` and `pick 100,250` must refer
                // to the same place, or every scripted pick misses. That is
                // the kernel's page space, y up from the bottom-left, which
                // is also the PDF's own convention. Pointer picks arrive in
                // app space instead, so this is the one place that converts,
                // through the module that owns the flip.
                let space = PageSpace::new(env.page_height);
                vec![Effect::TakePick(space.from_kernel(Vec2::new(at.x, at.y)))]
            }
        }
        Verb::Sensitivity(what) => vec![Effect::Sensitivity(what.clone())],
        Verb::FillSign(what) => vec![Effect::FillSign(what.clone())],
        Verb::PredefinedText(words) => vec![Effect::PredefinedText(words.clone())],
        Verb::EditObject | Verb::MoveThing | Verb::SignLine | Verb::SignRectangle
            if !env.has_doc =>
        {
            vec![Effect::SayError("nothing open.".into())]
        }
        Verb::EditObject => vec![Effect::TakeUpObjectTool { edit: true, page: env.page }],
        Verb::MoveThing => vec![Effect::TakeUpObjectTool { edit: false, page: env.page }],
        Verb::SignLine => vec![Effect::ArmSignLine { page: env.page }],
        Verb::SignRectangle => vec![Effect::ArmSignRectangle { page: env.page }],
        Verb::DocumentStatus => vec![Effect::ReportDocumentStatus],
        _ => return None,
    })
}

/// The signing verbs: the pad names and the draw-then-place decisions are
/// computed here from the library count.
fn plan_signing(verb: &Verb, env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        Verb::SessionLog => vec![Effect::ReportSessionLog],
        Verb::ApplySignatures => vec![Effect::RunApplySignatures],
        Verb::ManageSignatures(what) => vec![Effect::RunManageSignatures(what.clone())],
        Verb::Signature(SignatureAction::Draw) => {
            let name = if env.signature_count == 0 {
                "Signature".to_string()
            } else {
                format!("Signature {}", env.signature_count + 1)
            };
            vec![
                Effect::OpenSignaturePad { name, then_place: false },
                Effect::Say("draw a signature in the window.".into()),
            ]
        }
        Verb::Signature(SignatureAction::Upload(Some(path))) => {
            vec![Effect::UploadSignaturePath(path.clone())]
        }
        Verb::Signature(SignatureAction::Upload(None)) => vec![Effect::UploadSignatureDialog],
        // Nothing made yet needs one drawn before it can be placed; placing
        // does not.
        Verb::Signature(SignatureAction::Place) if env.signature_count == 0 => vec![
            Effect::OpenSignaturePad { name: "Signature".to_string(), then_place: true },
            Effect::Say(
                "draw your signature in the window — it is kept on this \
                 computer, and nowhere else. `signature upload` adds one \
                 from a picture instead."
                    .into(),
            ),
        ],
        Verb::Signature(SignatureAction::Place) if !env.has_doc => {
            vec![Effect::SayError("nothing open.".into())]
        }
        Verb::Signature(SignatureAction::Place) => vec![Effect::ArmSignature { page: env.page }],
        _ => return None,
    })
}

/// The security verbs: mirrors, with the password-state decisions still in
/// the app's bodies (they read live session state).
fn plan_security(verb: &Verb, _env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        Verb::Validate => vec![Effect::RunValidate],
        Verb::Certify(path) => vec![Effect::RunCertify(path.clone())],
        Verb::Whiteout => vec![Effect::ArmWhiteout],
        Verb::DrawArrow => vec![Effect::ArmDrawArrow],
        Verb::ToggleFill => vec![Effect::ToggleFill],
        Verb::Redact => vec![Effect::ArmRedact],
        Verb::Lock => vec![Effect::LockWords],
        Verb::Secure(options) => vec![Effect::RunSecure(*options)],
        Verb::SmartRedact { redact } => vec![Effect::RunSmartRedact { redact: *redact }],
        Verb::HiddenData { clean } => vec![Effect::RunHiddenData { clean: *clean }],
        Verb::Unsecure => vec![Effect::RunUnsecure],
        _ => return None,
    })
}

/// The file, text and find verbs.
fn plan_files_and_text(verb: &Verb, _env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        // Files: save/finish and the page-range operations.
        Verb::Finish => vec![Effect::FinishPick],
        Verb::Save => vec![Effect::SaveFile],
        Verb::SaveAs(path) => vec![Effect::SaveFileAs(path.clone())],
        Verb::SaveAsDialog => vec![Effect::SaveAsDialog],
        Verb::Extract { pages, dest } => {
            vec![Effect::RunExtract { pages: pages.clone(), dest: dest.clone() }]
        }
        Verb::Import { source, pages } => {
            vec![Effect::RunImport { source: source.clone(), pages: pages.clone() }]
        }
        Verb::DeletePages(pages) => vec![Effect::RunDeletePages(pages.clone())],
        Verb::InsertPage => vec![Effect::RunInsertPage],
        Verb::MovePages { pages, before } => {
            vec![Effect::RunMovePages { pages: pages.clone(), before: *before }]
        }
        Verb::ReplacePage { source, pages } => {
            vec![Effect::ReplacePage { source: source.clone(), pages: pages.clone() }]
        }
        Verb::Interleave { source, pages } => {
            vec![Effect::InterleavePages { source: source.clone(), pages: pages.clone() }]
        }
        Verb::ToImage { dest } => vec![Effect::ExportImages { dest: dest.clone() }],
        Verb::ToHtml { dest } => vec![Effect::ExportHtml { dest: dest.clone() }],
        Verb::Split { every, dest } => {
            vec![Effect::SplitPages { every: *every, dest: dest.clone() }]
        }
        Verb::WordCount { pages } => vec![Effect::ReportWordCount(pages.clone())],
        // Text and find.
        Verb::Reflow => vec![Effect::RunReflow],
        Verb::Find(needle) => vec![Effect::RunFind(needle.clone())],
        Verb::FindStep { forward } => vec![Effect::RunFindStep { forward: *forward }],
        Verb::Replace => vec![Effect::OpenFindReplace],
        Verb::Spelling => vec![Effect::OpenSpellCheck],
        Verb::Bookmark => vec![Effect::AddBookmark],
        Verb::ArticleBox => vec![Effect::ArmArticleBox],
        Verb::Weblinks => vec![Effect::BeginWebLink],
        Verb::JoinText => vec![Effect::BeginJoinText],
        Verb::MatchProperties => vec![Effect::BeginMatchProperties],
        // Copy is handled by the frame's own keyboard path, not by the
        // command executor; the verb plans to nothing.
        Verb::Copy => vec![],
        Verb::MarkText(kind) => vec![Effect::RunMarkText(*kind)],
        Verb::ListMarks => vec![Effect::RunListMarks],
        Verb::RemoveMark(n) => vec![Effect::RunRemoveMark(*n)],
        Verb::Note(text) => vec![Effect::RunNote(text.clone())],
        _ => return None,
    })
}

/// The documentation verbs — the Help tab's buttons: `about`, `shortcuts`,
/// `quickstart` and `reportissue` (the fifth, `checkupdate`, was already
/// built and needs no planning beyond the mirror).
fn plan_documentation(verb: &Verb, _env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        Verb::About => crate::verbs::about_text().into_iter().map(Effect::Say).collect(),
        Verb::QuickStart => crate::verbs::quickstart_text().into_iter().map(Effect::Say).collect(),
        Verb::Shortcuts => vec![Effect::ReportShortcuts],
        // The two things a report needs: where it goes, and the log of what
        // happened — `ReportSessionLog` says where that is (or why there is
        // none this run).
        Verb::ReportIssue => vec![
            Effect::Say(format!("found a problem? Report it at {}", crate::verbs::ISSUES_URL)),
            Effect::ReportSessionLog,
        ],
        _ => return None,
    })
}

/// The navigation and editing verbs — the last domain ported.
fn plan_navigation(verb: &Verb, _env: &CommandEnv) -> Option<Vec<Effect>> {
    Some(match verb {
        Verb::Undo => vec![Effect::UndoRedo { undo: true }],
        Verb::Redo => vec![Effect::UndoRedo { undo: false }],
        Verb::Pointer(mode) => vec![Effect::SetPointer(*mode)],
        Verb::CopyText => vec![Effect::CopyText],
        Verb::Paste => vec![Effect::Paste],
        Verb::EditText => vec![Effect::EditText],
        Verb::AddText(text) => vec![Effect::AddText(text.clone())],
        Verb::AddImage(path) => vec![Effect::AddImage(path.clone())],
        Verb::SetLayout(layout) => vec![Effect::SetLayout(layout.clone())],
        Verb::ReversePages => vec![Effect::RunReversePages],
        Verb::Thumbnails => vec![Effect::ToggleOrganizeGrid],
        Verb::ToggleAppearance => vec![Effect::ToggleAppearance],
        Verb::DuplicatePages(spec) => vec![Effect::RunDuplicatePages(spec.clone())],
        Verb::CropPages { pages, margin } => {
            vec![Effect::RunCropPages { pages: pages.clone(), margin: *margin }]
        }
        Verb::ResizePages { pages, width_pt, height_pt } => vec![Effect::RunResizePages {
            pages: pages.clone(),
            width_pt: *width_pt,
            height_pt: *height_pt,
        }],
        Verb::SwapPages { a, b } => vec![Effect::RunSwapPages { a: *a, b: *b }],
        Verb::RotatePages { pages, quarters } => {
            vec![Effect::RunRotatePages { pages: pages.clone(), quarters: *quarters }]
        }
        Verb::ExtractText(spec) => vec![Effect::RunExtractText(spec.clone())],
        Verb::OutlinedFont(action) => vec![Effect::RunOutlinedFont(action.clone())],
        Verb::ClearHistory => vec![Effect::ClearRecent],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbs::AppPointArg;
    use std::path::PathBuf;

    #[test]
    fn the_document_domain_plans_its_eleven_verbs() {
        let cases: Vec<(Verb, Vec<Effect>)> = vec![
            (Verb::Open(PathBuf::from("a.pdf")), vec![Effect::OpenPath(PathBuf::from("a.pdf"))]),
            (Verb::OpenDialog, vec![Effect::OpenFileDialog]),
            (Verb::Close { force: false }, vec![Effect::CloseDocument { force: false }]),
            (Verb::Close { force: true }, vec![Effect::CloseDocument { force: true }]),
            (Verb::Quit { force: false }, vec![Effect::Quit { force: false }]),
            (Verb::Quit { force: true }, vec![Effect::Quit { force: true }]),
            (Verb::Page(PageTarget::Next), vec![Effect::GoTo(PageTarget::Next)]),
            (Verb::Zoom(ZoomTarget::Fit), vec![Effect::Zoom(ZoomTarget::Fit)]),
            (Verb::RotatePage(90), vec![Effect::RotatePage(90)]),
            (Verb::TextLayer, vec![Effect::ReportTextLayer]),
            (Verb::Pdfium, vec![Effect::Say(crate::pdfium::describe())]),
            (Verb::Version, vec![Effect::Say(format!("Pagify {}", crate::VERSION))]),
            (
                Verb::CheckUpdate,
                vec![Effect::Say("checking for a newer build…".into()), Effect::CheckUpdate],
            ),
            (
                Verb::BlankDoc { width_pt: 612.0, height_pt: 792.0, ruling: pdf_core::document::Ruling::None },
                vec![Effect::BlankDocument { width_pt: 612.0, height_pt: 792.0, ruling: pdf_core::document::Ruling::None }],
            ),
            (
                Verb::Combine { folder: Some(std::path::PathBuf::from("in")) },
                vec![Effect::CombineFiles { folder: Some(std::path::PathBuf::from("in")) }],
            ),
            (Verb::AutoScroll, vec![Effect::ToggleAutoScroll]),
            (Verb::Ruler, vec![Effect::ToggleRuler]),
            (Verb::AccessCheck, vec![Effect::RunAccessCheck]),
            (
                Verb::AccessReport { dest: None },
                vec![Effect::WriteAccessReport { dest: None }],
            ),
            (Verb::AutoBookmarks, vec![Effect::RunAutoBookmarks]),
            (Verb::Preflight, vec![Effect::RunPreflight]),
            (
                Verb::ExportAllImages { dest: Some("out".into()) },
                vec![Effect::ExportPictures { dest: Some("out".into()) }],
            ),
        ];
        for (verb, want) in cases {
            assert_eq!(plan(&verb, &CommandEnv::default()), Some(want), "planning {verb:?}");
        }
        // The one document verb with a guard: there is nothing to capture
        // until a document is open.
        assert_eq!(
            plan(&Verb::Snapshot, &CommandEnv::default()),
            Some(vec![Effect::SayError("nothing open.".into())])
        );
        let with_doc = CommandEnv { has_doc: true, ..CommandEnv::default() };
        assert_eq!(plan(&Verb::Snapshot, &with_doc), Some(vec![Effect::Snapshot]));
    }

    #[test]
    fn the_objects_and_measurement_domains_plan_their_verbs() {
        use crate::verbs::MeasureKind;
        let cases: Vec<(Verb, Vec<Effect>)> = vec![
            (Verb::Layers, vec![Effect::ToggleLayers]),
            (Verb::RepairLocks, vec![Effect::RepairLocks]),
            (Verb::Opacity(40.0), vec![Effect::SetOpacity(40.0)]),
            (Verb::BringToFront, vec![Effect::Restack(Stacking::Front)]),
            (Verb::SendToBack, vec![Effect::Restack(Stacking::Back)]),
            (Verb::LockArea, vec![Effect::ArmLockArea]),
            (Verb::LockPages("1-3".into()), vec![Effect::LockPages("1-3".into())]),
            (Verb::Unlock, vec![Effect::Unlock]),
            (
                Verb::Calibrate { distance: 10.0, unit: "mm".into() },
                vec![Effect::ArmCalibrate { distance: 10.0, unit: "mm".into() }],
            ),
            (Verb::Scale, vec![Effect::ReportScale]),
            (Verb::Measure(MeasureKind::Distance), vec![Effect::ArmMeasure(MeasureKind::Distance)]),
            (Verb::Record("r".into()), vec![Effect::Record("r".into())]),
            (Verb::StopRecording, vec![Effect::StopRecording]),
            (
                Verb::Replay(std::path::PathBuf::from("r.json")),
                vec![Effect::Replay(std::path::PathBuf::from("r.json"))],
            ),
        ];
        for (verb, want) in cases {
            assert_eq!(plan(&verb, &CommandEnv::default()), Some(want), "planning {verb:?}");
        }
    }


    #[test]
    fn the_tools_domain_guards_and_flips_in_the_plan() {
        let no_doc = CommandEnv::default();
        let with_doc =
            CommandEnv { has_doc: true, page: 2, page_height: 800.0, tool_armed: true, signature_count: 3 };
        for verb in [Verb::EditObject, Verb::MoveThing, Verb::SignLine, Verb::SignRectangle] {
            assert_eq!(
                plan(&verb, &no_doc),
                Some(vec![Effect::SayError("nothing open.".into())]),
                "{verb:?} without a document"
            );
        }
        let pick = AppPointArg { x: 100.0, y: 200.0 };
        assert_eq!(
            plan(&Verb::Pick(pick), &CommandEnv { tool_armed: false, ..with_doc }),
            Some(vec![Effect::SayError("nothing is waiting for a click.".into())])
        );
        let want = PageSpace::new(800.0).from_kernel(Vec2::new(100.0, 200.0));
        assert_eq!(
            plan(&Verb::Pick(pick), &with_doc),
            Some(vec![Effect::TakePick(want)]),
            "the kernel-to-app flip matches the module the app used"
        );
        assert_eq!(
            plan(&Verb::EditObject, &with_doc),
            Some(vec![Effect::TakeUpObjectTool { edit: true, page: 2 }])
        );
        assert_eq!(
            plan(&Verb::MoveThing, &with_doc),
            Some(vec![Effect::TakeUpObjectTool { edit: false, page: 2 }])
        );
        assert_eq!(plan(&Verb::SignLine, &with_doc), Some(vec![Effect::ArmSignLine { page: 2 }]));
        assert_eq!(
            plan(&Verb::SignRectangle, &with_doc),
            Some(vec![Effect::ArmSignRectangle { page: 2 }])
        );
        assert_eq!(plan(&Verb::DocumentStatus, &with_doc), Some(vec![Effect::ReportDocumentStatus]));
    }


    #[test]
    fn the_signing_domain_plans_names_and_places() {
        use crate::verbs::SignatureAction;
        let no_sig = CommandEnv::default();
        let with_sig = CommandEnv {
            has_doc: true,
            page: 2,
            page_height: 800.0,
            tool_armed: false,
            signature_count: 3,
        };
        assert_eq!(
            plan(&Verb::Signature(SignatureAction::Draw), &no_sig),
            Some(vec![
                Effect::OpenSignaturePad { name: "Signature".into(), then_place: false },
                Effect::Say("draw a signature in the window.".into()),
            ])
        );
        assert_eq!(
            plan(&Verb::Signature(SignatureAction::Draw), &with_sig),
            Some(vec![
                Effect::OpenSignaturePad { name: "Signature 4".into(), then_place: false },
                Effect::Say("draw a signature in the window.".into()),
            ])
        );
        match plan(&Verb::Signature(SignatureAction::Place), &no_sig) {
            Some(v) => {
                assert!(matches!(v.as_slice(), [Effect::OpenSignaturePad { then_place: true, .. }, Effect::Say(_)]))
            }
            other => panic!("expected a pad hand-off, got {other:?}"),
        }
        assert_eq!(
            plan(&Verb::Signature(SignatureAction::Place), &with_sig),
            Some(vec![Effect::ArmSignature { page: 2 }])
        );
        let sig_no_doc = CommandEnv { signature_count: 3, ..CommandEnv::default() };
        assert_eq!(
            plan(&Verb::Signature(SignatureAction::Place), &sig_no_doc),
            Some(vec![Effect::SayError("nothing open.".into())])
        );
        assert_eq!(
            plan(
                &Verb::Signature(SignatureAction::Upload(Some(std::path::PathBuf::from("s.png")))),
                &no_sig
            ),
            Some(vec![Effect::UploadSignaturePath(std::path::PathBuf::from("s.png"))])
        );
        assert_eq!(
            plan(&Verb::Signature(SignatureAction::Upload(None)), &no_sig),
            Some(vec![Effect::UploadSignatureDialog])
        );
        assert_eq!(plan(&Verb::SessionLog, &no_sig), Some(vec![Effect::ReportSessionLog]));
    }

    #[test]
    fn the_security_domain_mirrors_its_verbs() {
        let env = CommandEnv::default();
        let cases: Vec<(Verb, Vec<Effect>)> = vec![
            (Verb::Validate, vec![Effect::RunValidate]),
            (Verb::Certify(None), vec![Effect::RunCertify(None)]),
            (Verb::Whiteout, vec![Effect::ArmWhiteout]),
            (Verb::DrawArrow, vec![Effect::ArmDrawArrow]),
            (Verb::ToggleFill, vec![Effect::ToggleFill]),
            (Verb::Redact, vec![Effect::ArmRedact]),
            (Verb::Lock, vec![Effect::LockWords]),
            (Verb::SmartRedact { redact: true }, vec![Effect::RunSmartRedact { redact: true }]),
            (Verb::HiddenData { clean: false }, vec![Effect::RunHiddenData { clean: false }]),
            (Verb::Unsecure, vec![Effect::RunUnsecure]),
        ];
        for (verb, want) in cases {
            assert_eq!(plan(&verb, &env), Some(want), "planning {verb:?}");
        }
    }

    #[test]
    fn the_files_and_text_domains_mirror_their_verbs() {
        use crate::verbs::Markup;
        let env = CommandEnv::default();
        let cases: Vec<(Verb, Vec<Effect>)> = vec![
            (Verb::Finish, vec![Effect::FinishPick]),
            (Verb::Save, vec![Effect::SaveFile]),
            (Verb::SaveAs("a.pdf".into()), vec![Effect::SaveFileAs("a.pdf".into())]),
            (Verb::SaveAsDialog, vec![Effect::SaveAsDialog]),
            (
                Verb::Extract { pages: "1-2".into(), dest: "o.pdf".into() },
                vec![Effect::RunExtract { pages: "1-2".into(), dest: "o.pdf".into() }],
            ),
            (
                Verb::Import { source: "i.pdf".into(), pages: "1".into() },
                vec![Effect::RunImport { source: "i.pdf".into(), pages: "1".into() }],
            ),
            (Verb::DeletePages("1".into()), vec![Effect::RunDeletePages("1".into())]),
            (Verb::InsertPage, vec![Effect::RunInsertPage]),
            (
                Verb::MovePages { pages: "2".into(), before: 0 },
                vec![Effect::RunMovePages { pages: "2".into(), before: 0 }],
            ),
            (
                Verb::ReplacePage { source: Some("o.pdf".into()), pages: "1".into() },
                vec![Effect::ReplacePage { source: Some("o.pdf".into()), pages: "1".into() }],
            ),
            (
                Verb::Interleave { source: None, pages: "all".into() },
                vec![Effect::InterleavePages { source: None, pages: "all".into() }],
            ),
            (
                Verb::ToImage { dest: None },
                vec![Effect::ExportImages { dest: None }],
            ),
            (
                Verb::ToHtml { dest: Some("book.html".into()) },
                vec![Effect::ExportHtml { dest: Some("book.html".into()) }],
            ),
            (
                Verb::Split { every: 2, dest: None },
                vec![Effect::SplitPages { every: 2, dest: None }],
            ),
            (Verb::WordCount { pages: String::new() }, vec![Effect::ReportWordCount(String::new())]),
            (Verb::Reflow, vec![Effect::RunReflow]),
            (Verb::Find("n".into()), vec![Effect::RunFind("n".into())]),
            (Verb::FindStep { forward: true }, vec![Effect::RunFindStep { forward: true }]),
            (Verb::Replace, vec![Effect::OpenFindReplace]),
            (Verb::Spelling, vec![Effect::OpenSpellCheck]),
            (Verb::Bookmark, vec![Effect::AddBookmark]),
            (Verb::ArticleBox, vec![Effect::ArmArticleBox]),
            (Verb::Weblinks, vec![Effect::BeginWebLink]),
            (Verb::JoinText, vec![Effect::BeginJoinText]),
            (Verb::MatchProperties, vec![Effect::BeginMatchProperties]),
            (Verb::Copy, vec![]),
            (Verb::MarkText(Markup::Highlight), vec![Effect::RunMarkText(Markup::Highlight)]),
            (Verb::ListMarks, vec![Effect::RunListMarks]),
            (Verb::RemoveMark(2), vec![Effect::RunRemoveMark(2)]),
            (Verb::Note("t".into()), vec![Effect::RunNote("t".into())]),
        ];
        for (verb, want) in cases {
            assert_eq!(plan(&verb, &env), Some(want), "planning {verb:?}");
        }
    }
    #[test]
    fn the_navigation_domain_mirrors_its_verbs() {
        use crate::verbs::PointerMode;
        let env = CommandEnv::default();
        let cases: Vec<(Verb, Vec<Effect>)> = vec![
            (Verb::Undo, vec![Effect::UndoRedo { undo: true }]),
            (Verb::Redo, vec![Effect::UndoRedo { undo: false }]),
            (Verb::Pointer(PointerMode::Pan), vec![Effect::SetPointer(PointerMode::Pan)]),
            (Verb::CopyText, vec![Effect::CopyText]),
            (Verb::Paste, vec![Effect::Paste]),
            (Verb::EditText, vec![Effect::EditText]),
            (Verb::AddText("hi".into()), vec![Effect::AddText("hi".into())]),
            (Verb::AddImage(None), vec![Effect::AddImage(None)]),
            (Verb::ReversePages, vec![Effect::RunReversePages]),
            (Verb::Thumbnails, vec![Effect::ToggleOrganizeGrid]),
            (Verb::ToggleAppearance, vec![Effect::ToggleAppearance]),
            (Verb::DuplicatePages("1".into()), vec![Effect::RunDuplicatePages("1".into())]),
            (
                Verb::CropPages { pages: "1".into(), margin: 4.0 },
                vec![Effect::RunCropPages { pages: "1".into(), margin: 4.0 }],
            ),
            (
                Verb::ResizePages { pages: "1".into(), width_pt: 612.0, height_pt: 792.0 },
                vec![Effect::RunResizePages { pages: "1".into(), width_pt: 612.0, height_pt: 792.0 }],
            ),
            (Verb::SwapPages { a: 0, b: 1 }, vec![Effect::RunSwapPages { a: 0, b: 1 }]),
            (
                Verb::RotatePages { pages: "1".into(), quarters: 1 },
                vec![Effect::RunRotatePages { pages: "1".into(), quarters: 1 }],
            ),
            (Verb::ExtractText("all".into()), vec![Effect::RunExtractText("all".into())]),
            (Verb::ClearHistory, vec![Effect::ClearRecent]),
        ];
        for (verb, want) in cases {
            assert_eq!(plan(&verb, &env), Some(want), "planning {verb:?}");
        }
    }

    #[test]
    fn the_documentation_domain_plans_its_verbs() {
        let env = CommandEnv::default();
        match plan(&Verb::About, &env) {
            Some(effects) => assert!(
                matches!(effects.as_slice(), [Effect::Say(_), Effect::Say(_)]),
                "about planned {effects:?}"
            ),
            other => panic!("about planned {other:?}"),
        }
        assert_eq!(plan(&Verb::Shortcuts, &env), Some(vec![Effect::ReportShortcuts]));
        assert_eq!(
            plan(&Verb::QuickStart, &env).map(|effects| effects.len()),
            Some(crate::verbs::quickstart_text().len())
        );
        assert_eq!(
            plan(&Verb::ReportIssue, &env),
            Some(vec![
                Effect::Say(format!(
                    "found a problem? Report it at {}",
                    crate::verbs::ISSUES_URL
                )),
                Effect::ReportSessionLog,
            ])
        );
    }

    #[test]
    fn only_help_and_planned_have_no_plan() {
        // These two never reach `act`: `run()` answers them itself.
        assert!(plan(&Verb::Help(None), &CommandEnv::default()).is_none());
        assert!(plan(&Verb::Planned { verb: "x".into(), phase: "later".into() }, &CommandEnv::default()).is_none());
    }
}
