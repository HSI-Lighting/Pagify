use crate::{hub, icon_font, theme, ToolId};
use pagify_shell::command::Dispatch;

/// What a ribbon button's own third field means: its literal dispatch
/// text always, and, where it corresponds to one of `Tool::id`'s own
/// identities, the `ToolId` itself — DESIGN_REVIEW.md §2.8.4 /
/// `Pagify-Phase2-BigTasks.md` §4, finishing what `Tool::id`/`ToolId`
/// started. `armed` is compared against this by `ToolId` equality now
/// (`lit_by`), not by a string that could drift between this table and
/// `Tool::id`'s own match.
///
/// **Carries the dispatch text itself, trailing space and all.**
/// `Calibrate`'s own button fills the box and waits ("calibrate ",
/// trailing space, see `ribbon_click`'s own "ends with a space" rule) —
/// a rendering quirk of that one button, not a second identity to keep
/// in sync by hand.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Command {
    Tool(ToolId, &'static str),
    /// Everything else — an ordinary verb with no `Tool` behind it at all
    /// (`"save"`, `"import"`, …), or a tool-shaped command with no
    /// ribbon-lit identity of its own (`Draw(Rectangle)`'s `"rectangle"`,
    /// `PlaceImage`, every `tools::Op` verb like `fillet`/`trim`/`offset`).
    Verb(&'static str),
}

impl Command {
    pub(crate) fn text(&self) -> &'static str {
        match self {
            Command::Tool(_, text) | Command::Verb(text) => text,
        }
    }

    /// Whether `armed` is this button's own tool. Always `false` for a
    /// `Verb` — it has no `ToolId` to be lit by in the first place.
    pub(crate) fn lit_by(&self, armed: Option<ToolId>) -> bool {
        matches!(self, Command::Tool(id, _) if Some(*id) == armed)
    }
}

/// One ribbon button: the glyph, the name under it, and the command it runs.
///
/// A command string rather than a callback, because §7 makes the command box
/// the single way anything happens — a button that did something the box could
/// not would be a second, undiscoverable interface.
pub type Tool = (&'static str, &'static str, Command);

/// Hand and Select, which the reference toolbar repeats at the head of every
/// tab.
///
/// Written once and rendered on each tab rather than copied into all fifteen:
/// they are the same two tools, and fifteen copies is fifteen chances for them
/// to drift apart. The File tab is the exception — it is a backstage, not a
/// toolbar.
const ALWAYS: &[Tool] = &[
    ("\u{E925}", "Hand", Command::Verb("hand")),
    ("\u{EF52}", "Select", Command::Verb("selecttool")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
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
    pub(crate) const ALL: [Tab; 15] = [
        Tab::File, Tab::Home, Tab::Edit, Tab::Convert,
        Tab::Organize, Tab::Comment, Tab::View, Tab::Form,
        Tab::Protect, Tab::PagiSign, Tab::Share, Tab::Accessibility,
        Tab::Help, Tab::Draw, Tab::Automate,
    ];

    pub(crate) fn label(self) -> &'static str {
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
    pub(crate) fn leading(self) -> &'static [Tool] {
        if self == Tab::File { &[] } else { ALWAYS }
    }

    pub(crate) fn buttons(self) -> &'static [Tool] {
        match self {
            Tab::File => &[
                ("\u{E2C8}", "Open…", Command::Verb("open")),
                ("\u{E5CD}", "Close", Command::Verb("close")),
                ("\u{E161}", "Save", Command::Verb("save")),
                ("\u{E161}", "Save As…", Command::Verb("saveas")),
                ("\u{E8B8}", "PDFium", Command::Verb("pdfium")),
                ("\u{F8C7}", "Quit", Command::Verb("quit")),
            ],
            Tab::Home => &[
                ("\u{E412}", "Snapshot", Command::Verb("snapshot")),
                ("\u{E14D}", "Copy", Command::Verb("copy")),
                ("\u{E8E7}", "Bookmark", Command::Verb("bookmark")),
                ("\u{E8FF}", "Zoom In", Command::Verb("zoom in")),
                ("\u{E900}", "Zoom Out", Command::Verb("zoom out")),
                ("\u{EA10}", "Fit Page", Command::Verb("zoom fit")),
                ("\u{F779}", "Fit Width", Command::Verb("zoom width")),
                ("\u{E3F4}", "Actual Size", Command::Verb("zoom actual")),
                ("\u{E5FA}", "Extract Text", Command::Verb("extracttext")),
                ("\u{E41A}", "Rotate View", Command::Verb("rotate")),
                ("\u{E262}", "Edit Text", Command::Tool(ToolId::EditText, "edittext")),
                ("\u{E162}", "Edit Object", Command::Verb("editobject")),
                ("\u{E53B}", "Layers", Command::Verb("layers")),
                ("\u{E883}", "Bring to Front", Command::Verb("bringtofront")),
                ("\u{E882}", "Send to Back", Command::Verb("sendtoback")),
                ("\u{E89F}", "Move", Command::Verb("moveobject")),
                ("\u{F82B}", "Highlight", Command::Tool(ToolId::Highlight, "highlight")),
                ("\u{E418}", "Rotate Pages", Command::Verb("rotatepages")),
                ("\u{E145}", "Insert", Command::Verb("insertpage")),
                ("\u{E329}", "From Scanner", Command::Verb("fromscanner")),
                ("\u{E746}", "Fill & Sign", Command::Verb("fillsign")),
                // The two a form actually asks for, one press away.
                ("\u{E668}", "Tick", Command::Verb("fillsign tick")),
                ("\u{E5CD}", "Cross", Command::Verb("fillsign cross")),
            ],
            Tab::Convert => &[
                ("\u{E873}", "From Files", Command::Verb("fromfiles")),
                ("\u{E329}", "From Scanner", Command::Verb("fromscanner")),
                ("\u{E14F}", "From Clipboard", Command::Verb("fromclipboard")),
                ("\u{E0EE}", "Form", Command::Verb("createform")),
                ("\u{EBBD}", "PDF Portfolio", Command::Verb("portfolio")),
                ("\u{EB98}", "Combine Files", Command::Verb("combine")),
                ("\u{E66D}", "Blank", Command::Verb("blankdoc")),
                ("\u{E99B}", "From Template", Command::Verb("fromtemplate")),
                ("\u{EFA2}", "Export All Images", Command::Verb("exportimages")),
                ("\u{F1BE}", "To MS Office", Command::Verb("tooffice")),
                ("\u{E3F4}", "To Image", Command::Verb("toimage")),
                ("\u{EB7E}", "To HTML", Command::Verb("tohtml")),
                ("\u{F720}", "To Other", Command::Verb("toother")),
                ("\u{F0C5}", "Preflight", Command::Verb("preflight")),
            ],
            Tab::Edit => &[
                ("\u{E262}", "Edit Text", Command::Tool(ToolId::EditText, "edittext")),
                ("\u{E162}", "Edit Object", Command::Verb("editobject")),
                ("\u{E236}", "Link & Join Text", Command::Verb("jointext")),
                // Drag the sample text, then drag whatever needs to match it
                // — see `begin_match_properties`'s own doc for the two-step
                // shape and why the tool stays in hand between selections.
                ("\u{E262}", "Match Properties", Command::Tool(ToolId::MatchProperties, "matchproperties")),
                ("\u{E8CE}", "Check Spelling", Command::Verb("spelling")),
                ("\u{E881}", "Search & Replace", Command::Verb("replace")),
                // Bare, not pre-filled: this drags out a box to type into
                // (see `begin_text_box`), the same click-and-place shape Add
                // Images already has, rather than asking for the words in
                // the command line first.
                ("\u{EAE2}", "Add Text", Command::Tool(ToolId::AddText, "addtext")),
                ("\u{E43E}", "Add Images", Command::Verb("addimage")),
                ("\u{E8EC}", "Add Article Box", Command::Tool(ToolId::ArticleBox, "articlebox")),
                ("\u{EA07}", "Web Links", Command::Tool(ToolId::Link, "weblinks")),
                ("\u{E8E7}", "Bookmark", Command::Verb("bookmark")),
                ("\u{F184}", "Cross Reference", Command::Verb("crossref")),
            ],
            Tab::Organize => &[
                ("\u{E9B0}", "Thumbnail View", Command::Verb("thumbnails")),
                ("\u{E145}", "Insert", Command::Verb("insertpage")),
                ("\u{E92E}", "Delete", Command::Verb("deletepage")),
                ("\u{E14E}", "Extract", Command::Verb("extract")),
                ("\u{E8D5}", "Reverse", Command::Verb("reversepages")),
                ("\u{E8FE}", "Rearrange", Command::Verb("rearrange")),
                ("\u{E89F}", "Move", Command::Verb("movepage")),
                ("\u{E173}", "Duplicate", Command::Verb("duplicatepage")),
                ("\u{F232}", "Replace", Command::Verb("replacepage")),
                ("\u{E0B6}", "Split", Command::Verb("split")),
                ("\u{E8D4}", "Swap", Command::Verb("swappages ")),
                ("\u{EAF4}", "Interleaving", Command::Verb("interleave")),
                ("\u{E418}", "Rotate Pages", Command::Verb("rotatepages")),
                ("\u{E3BE}", "Crop Pages", Command::Verb("croppages all ")),
                ("\u{E85B}", "Resize Pages", Command::Verb("resizepages all ")),
                ("\u{E53C}", "Flatten", Command::Verb("flatten")),
                ("\u{E41C}", "Page Marks", Command::Verb("pagemarks")),
            ],
            Tab::Comment => &[
                ("\u{F82B}", "Highlight", Command::Tool(ToolId::Highlight, "highlight")),
                ("\u{E249}", "Underline", Command::Tool(ToolId::Underline, "underline")),
                ("\u{E246}", "Strikeout", Command::Tool(ToolId::StrikeOut, "strikeout")),
                ("\u{E155}", "Squiggly", Command::Tool(ToolId::Squiggly, "squiggly")),
                ("\u{E0D7}", "Replace Text", Command::Verb("replacetext")),
                ("\u{F735}", "Insert Text", Command::Verb("inserttext")),
                ("\u{F1FC}", "Note", Command::Verb("note ")),
                ("\u{E2BC}", "File", Command::Verb("attachcomment")),
                ("\u{E312}", "Typewriter", Command::Tool(ToolId::AddText, "addtext")),
                ("\u{E3BC}", "Textbox", Command::Verb("textbox")),
                ("\u{E0CB}", "Callout", Command::Verb("callout")),
                ("\u{EBBB}", "Drawing", Command::Verb("drawing")),
                ("\u{F097}", "Pencil", Command::Verb("pencil")),
                ("\u{E6D0}", "Eraser", Command::Tool(ToolId::EraseMark, "erase")),
                ("\u{E162}", "Area Highlight", Command::Verb("areahighlight")),
                ("\u{F02F}", "Search & Highlight", Command::Verb("searchhighlight")),
                ("\u{EA5F}", "Accounting Calculator", Command::Verb("calculator")),
                ("\u{EA49}", "Measure", Command::Tool(ToolId::MeasureDistance, "measure distance")),
                ("\u{E982}", "Stamp", Command::Verb("stamp")),
                ("\u{EF76}", "Custom Stamp", Command::Verb("customstamp")),
                ("\u{E668}", "TickMark", Command::Verb("tickmark")),
                ("\u{E8AF}", "Manage Comments", Command::Verb("managecomments")),
                ("\u{F10D}", "Keep Tool Selected", Command::Verb("keeptool")),
            ],
            Tab::View => &[
                // `\u{E793}` (Segoe Fluent's own "DarkTheme" glyph) was tried
                // first and failed `every_ribbon_glyph_can_actually_be_drawn`
                // — not in this font's own subset. This codepoint is
                // confirmed present (same test), picked from the font's
                // actual coverage rather than guessed a second time; its
                // exact pictograph wasn't verified beyond that, so swap it
                // for another confirmed-present codepoint if it doesn't read
                // as "appearance" on screen.
                ("\u{E226}", "Light Theme", Command::Verb("appearance")),
                ("\u{E40A}", "Change Color", Command::Verb("changecolor")),
                ("\u{E3E8}", "Reverse View", Command::Verb("reverseview")),
                ("\u{E41A}", "Rotate View", Command::Verb("rotate")),
                ("\u{E41C}", "Toggle Ruler", Command::Verb("ruler")),
                ("\u{E3C5}", "Single Page", Command::Verb("viewsingle")),
                ("\u{E0E0}", "Facing", Command::Verb("viewfacing")),
                ("\u{E8ED}", "Continuous", Command::Verb("viewcontinuous")),
                ("\u{E666}", "Continuous Facing", Command::Verb("viewcontinuousfacing")),
                ("\u{E86E}", "Separate Cover", Command::Verb("viewcover")),
                ("\u{E0B6}", "Split", Command::Verb("viewsplit")),
                ("\u{E235}", "Reflow", Command::Verb("reflow")),
                ("\u{E71C}", "Page Transitions", Command::Verb("transitions")),
                ("\u{E258}", "AutoScroll", Command::Verb("autoscroll")),
                ("\u{E87A}", "Assistant", Command::Verb("assistant")),
                ("\u{E050}", "Read", Command::Verb("readaloud")),
                ("\u{E3B9}", "Compare", Command::Verb("compare")),
                ("\u{EAC7}", "Word Count", Command::Verb("wordcount")),
                ("\u{E429}", "View Setting", Command::Verb("viewsetting")),
            ],
            Tab::Form => &[
                ("\u{F04C}", "Run Form Field Recognition", Command::Verb("formrecognise")),
                ("\u{F10A}", "Designer Assistant", Command::Verb("formdesigner")),
                ("\u{F1C1}", "Push Button", Command::Verb("fieldbutton")),
                ("\u{E9DE}", "Check Box", Command::Verb("fieldcheckbox")),
                ("\u{E837}", "Radio Button", Command::Verb("fieldradio")),
                ("\u{E262}", "Text Field", Command::Verb("fieldtext")),
                ("\u{E896}", "List Box", Command::Verb("fieldlist")),
                ("\u{E5C6}", "Combo Box", Command::Verb("fieldcombo")),
                ("\u{E3F4}", "Image Field", Command::Verb("fieldimage")),
                ("\u{EBCC}", "Date Field", Command::Verb("fielddate")),
                ("\u{F74C}", "Signature Field", Command::Verb("fieldsignature")),
                ("\u{E70B}", "Barcode", Command::Verb("fieldbarcode")),
                ("\u{E66B}", "Page Templates", Command::Verb("pagetemplates")),
                ("\u{F88C}", "Edit Static XFA Form", Command::Verb("editxfa")),
                ("\u{E24A}", "Calculation Order", Command::Verb("calcorder")),
                ("\u{E8FD}", "Add Tooltip", Command::Verb("tooltip")),
                ("\u{F053}", "Reset Form", Command::Verb("resetform")),
                ("\u{E265}", "Form to sheet", Command::Verb("formtosheet")),
                ("\u{F090}", "Import", Command::Verb("formimport")),
                ("\u{F09B}", "Export", Command::Verb("formexport")),
                ("\u{E86F}", "JavaScript", Command::Verb("javascript")),
                ("\u{E8B9}", "Tool Settings", Command::Verb("toolsettings")),
            ],
            Tab::Protect => &[
                // First, because it is the one that works and the one the tab is
                // for. "Mark for Redaction" beside it would be two names for the
                // same intention where only one of them destroys anything.
                ("\u{E243}", "Redact", Command::Tool(ToolId::Redact, "redact")),
                // Named beside Redact rather than under Secure Document, because
                // the choice between them is the one a user actually makes and
                // they need to be read together. Lock hides; Redact destroys.
                ("\u{E63F}", "Lock Text", Command::Tool(ToolId::Lock, "lock")),
                ("\u{E3C2}", "Lock Area", Command::Verb("lockarea")),
                // Beside Lock Area because it is the same promise at a
                // different scale, and because area locking refuses the scanned
                // and outlined pages this one takes — a reader turned away by
                // the first needs to see the second without hunting for it.
                ("\u{F686}", "Lock Pages", Command::Verb("lock all")),
                ("\u{E898}", "Unlock", Command::Verb("unlock")),
                // Grouped with the three above rather than after the stubs:
                // this is what makes Redact and Lock Area actually succeed on
                // a page whose words are drawn as outlines instead of falling
                // back to slow OCR — the tool a reader needs at the exact
                // moment one of those two does not work the way they expect.
                ("\u{E167}", "Outlined-Text Fonts", Command::Verb("outlinedfont")),
                ("\u{E8F5}", "Smart Redact", Command::Verb("smartredact")),
                // Beside it, because finding is the safe half and acting is
                // the one people came for — and because redaction destroys.
                ("\u{F74F}", "Redact Found", Command::Verb("smartredact redact")),
                ("\u{E23B}", "Whiteout", Command::Tool(ToolId::Whiteout, "whiteout")),
                ("\u{EA17}", "Hidden Data", Command::Verb("hiddendata")),
                // Beside the survey rather than hidden behind it: reporting is
                // the safe half and removing is the one people came for.
                ("\u{E16C}", "Remove Hidden Data", Command::Verb("hiddendata clean")),
                ("\u{E899}", "Secure Document", Command::Verb("secure")),
                // Beside it because it is the same action with the permissions
                // turned down, and because a reader who wants "they can read it
                // but not lift the artwork" should not have to find the words.
                ("\u{E593}", "Secure Read-only", Command::Verb("secure readonly")),
                ("\u{F03F}", "Remove Password", Command::Verb("unsecure")),
                ("\u{F0C6}", "Sensitivity", Command::Verb("sensitivity")),
                // The one most people reach for, one press away — and the verb
                // takes the others.
                ("\u{E948}", "Mark Confidential", Command::Verb("sensitivity confidential")),
                ("\u{E746}", "Fill & Sign", Command::Verb("fillsign")),
                ("\u{E7AF}", "Sign & Certify", Command::Verb("certify")),
                ("\u{F013}", "Validate", Command::Verb("validate")),
            ],
            Tab::PagiSign => &[
                ("\u{F603}", "Signature", Command::Tool(ToolId::Signature, "signature")),
                ("\u{E43E}", "Upload Signature", Command::Verb("signature upload")),
                ("\u{F775}", "Manage Signatures", Command::Verb("managesignatures")),
                ("\u{E877}", "Apply All Signatures", Command::Verb("applysignatures")),
                ("\u{EAE2}", "Add Text", Command::Tool(ToolId::AddText, "addtext")),
                ("\u{E8F3}", "Comb Field", Command::Verb("combfield")),
                ("\u{EF6C}", "Predefined Text", Command::Verb("predefinedtext")),
                ("\u{EB54}", "Rectangle", Command::Tool(ToolId::SignRectangle, "signrectangle")),
                ("\u{E668}", "Check", Command::Verb("signcheck")),
                ("\u{EF4A}", "Dot", Command::Verb("signdot")),
                ("\u{E5CD}", "Cross", Command::Verb("signcross")),
                ("\u{F108}", "Line", Command::Tool(ToolId::SignLine, "signline")),
                ("\u{F0D2}", "Request Signature", Command::Verb("requestsignature")),
                ("\u{F187}", "Send in Bulk", Command::Verb("sendbulk")),
                ("\u{F728}", "Create Online Form", Command::Verb("onlineform")),
                ("\u{EF3E}", "Document Status", Command::Verb("documentstatus")),
                ("\u{E06B}", "Add E-Sign Branding", Command::Verb("signbranding")),
            ],
            Tab::Share => &[
                ("\u{E159}", "Email", Command::Verb("email")),
                ("\u{EA5E}", "Attach to Email", Command::Verb("emailattach")),
                ("\u{E80D}", "Share Link", Command::Verb("sharelink")),
                ("\u{E560}", "Send for Review", Command::Verb("sendreview")),
                ("\u{E8E1}", "Track Reviews", Command::Verb("trackreviews")),
                ("\u{F15C}", "Cloud Storage", Command::Verb("cloudstorage")),
            ],
            Tab::Accessibility => &[
                ("\u{E6B1}", "Full Check", Command::Verb("accesscheck")),
                ("\u{F071}", "Accessibility Report", Command::Verb("accessreport")),
                ("\u{E893}", "Autotag Document", Command::Verb("autotag")),
                ("\u{E242}", "Reading Order", Command::Verb("readingorder")),
                ("\u{E43F}", "Set Alternate Text", Command::Verb("alttext")),
                ("\u{F05B}", "Tags Panel", Command::Verb("tagspanel")),
                ("\u{E92C}", "Reading Options", Command::Verb("readingoptions")),
            ],
            Tab::Help => &[
                ("\u{EA19}", "User Manual", Command::Verb("help")),
                ("\u{EB9B}", "Quick Start", Command::Verb("quickstart")),
                ("\u{EAE7}", "Keyboard Shortcuts", Command::Verb("shortcuts")),
                ("\u{E923}", "Check for Updates", Command::Verb("checkupdates")),
                ("\u{E868}", "Report an Issue", Command::Verb("reportissue")),
                ("\u{E88E}", "About Pagify", Command::Verb("about")),
            ],
            Tab::Draw => &[
                ("\u{F108}", "Line", Command::Tool(ToolId::Line, "line")),
                ("\u{EF4A}", "Circle", Command::Tool(ToolId::Circle, "circle")),
                ("\u{EB54}", "Rectangle", Command::Verb("rectangle")),
                // Chosen before drawing: whether Rectangle and Circle come out
                // filled solid or hollow. Lit up while on, like every other
                // standing choice on this ribbon.
                ("\u{F82B}", "Fill", Command::Verb("fill")),
                ("\u{E922}", "Polyline", Command::Tool(ToolId::Polyline, "pline")),
                ("\u{E5C8}", "Arrow", Command::Tool(ToolId::Arrow, "arrow")),
                ("\u{F757}", "Spline", Command::Tool(ToolId::Spline, "spline")),
                ("\u{E14E}", "Trim", Command::Verb("trim")),
                ("\u{E920}", "Fillet", Command::Verb("fillet")),
                ("\u{E2EC}", "Offset", Command::Verb("offset")),
                ("\u{E6D0}", "Erase", Command::Tool(ToolId::EraseMark, "erase")),
                ("\u{E41C}", "Calibrate", Command::Tool(ToolId::Calibrate, "calibrate ")),
                ("\u{EB95}", "Distance", Command::Tool(ToolId::MeasureDistance, "measure distance")),
                ("\u{EA49}", "Area", Command::Tool(ToolId::MeasureArea, "measure area")),
                ("\u{EAF6}", "Page Scale", Command::Verb("pagescale")),
            ],
            Tab::Automate => &[
                ("\u{E837}", "Record", Command::Verb("record")),
                ("\u{EF71}", "Stop", Command::Verb("stop")),
                ("\u{E037}", "Replay", Command::Verb("replay ")),
            ],
        }
    }

    /// Where a tab's own tools cluster into visually separated groups — the
    /// index into [`Self::buttons`] each later group starts at, a divider
    /// drawn before it. See `../docs/COMPACT_UI_RESTYLE.md` §4: only `Draw`
    /// gets this, since it's the one tab whose tools are a close content
    /// match for the mockup's own grouped clusters (shapes / edit / measure)
    /// — no other tab has a reference for how *its* tools would cluster.
    pub(crate) fn button_group_starts(self) -> &'static [usize] {
        match self {
            // 0 Line, 1 Circle, 2 Rectangle, 3 Fill, 4 Polyline, 5 Arrow,
            // 6 Spline | 7 Trim, 8 Fillet, 9 Offset, 10 Erase | 11 Calibrate,
            // 12 Distance, 13 Area, 14 Page Scale.
            Tab::Draw => &[7, 11],
            _ => &[],
        }
    }
}

/// What a click on a ribbon button or a Tool Wizard card does with the command
/// string it carries. Both reach the one handler in `ui`, so this is the one
/// place that decides, and a button added later is covered without a line of
/// its own.
///
/// **Reported from use (report 10): Extract "lacks a UI, only the raw command
/// works".** Delete, Extract and Move — and the Home cards for Merge and
/// Extract — ran their bare command at once and met a red usage error with the
/// box emptied: the table's only convention was "a trailing space means wait",
/// and nothing checked that a bare command *could* succeed. The parser already
/// knows: it answers a bare command that needs more with `Dispatch::Bad`, so
/// that is asked rather than a second hand-kept list of verbs that would drift
/// from it (`every_ribbon_button_runs_a_command_the_box_understands` exists
/// because two such lists already had).
#[derive(Debug, PartialEq)]
pub(crate) enum RibbonClick {
    /// Run it now, as written.
    Run,
    /// Put it in the box with the caret at the end and wait for the rest, with
    /// the usage line when the bare command was refused.
    Fill(Option<String>),
    /// A verb that takes a file: ask for the file the way bare `open` does,
    /// instead of filling a path in by hand.
    PickFile,
    /// `extract`: it needs pages *and* a file, so both are asked for in a
    /// dialog — see [`ExtractAsk`].
    Extract,
}

pub(crate) fn ribbon_click(command: &str) -> RibbonClick {
    // The table's own "fill and wait" buttons (Swap, Note, Calibrate…).
    if command.ends_with(' ') {
        return RibbonClick::Fill(None);
    }
    match pagify_shell::command::dispatch(command) {
        Some(Dispatch::Bad(_)) if command.split_whitespace().next() == Some("import") => {
            RibbonClick::PickFile
        }
        Some(Dispatch::Bad(_)) if command.split_whitespace().next() == Some("extract") => {
            RibbonClick::Extract
        }
        Some(Dispatch::Bad(usage)) => RibbonClick::Fill(Some(usage)),
        _ => RibbonClick::Run,
    }
}

pub(crate) fn tool_button(
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
                theme::violet_deep(),
                egui::Stroke::new(1.0, theme::violet_bright()),
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
            painter.layout_no_wrap(glyph.to_owned(), icon_font(19.0), theme::ink());
        // Wrapped inside the button rather than allowed to set its own width,
        // so every button in the row stays the same size.
        let label_galley = painter.layout(
            label.to_owned(),
            egui::FontId::proportional(10.0),
            if active || response.hovered() { theme::ink() } else { theme::ink_dim() },
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
                theme::ink(),
            );
        }
    }

    // What to type to get the same thing. §7 makes the command box the only way
    // anything happens, so a button that never says its own name leaves the box
    // undiscoverable to anyone who only ever clicks.
    response.on_hover_text(format!("{label}  —  `{}`", command.trim()))
}

/// Narrowed from the original 82 toward the compact mockup's own density
/// (`../docs/COMPACT_UI_RESTYLE.md` §4) — not all the way to it, because this
/// app's longest labels ("Run Form Field Recognition") aren't in the
/// mockup's own, shorter set and need the room. `no_ribbon_label_overflows_
/// its_own_button` is what actually found the floor: 60 wide clips several
/// three-word labels regardless of height, so the width stayed at 66 and
/// only `TOOL_HEIGHT` was tried lower — which clips two labels by a few
/// pixels at 54, so it stayed at its original 58 rather than fixing a width
/// problem by making every other button taller than it needs to be.
pub(crate) const TOOL_WIDTH: f32 = 66.0;
pub(crate) const TOOL_HEIGHT: f32 = 58.0;

/// The ribbon strip's own inner margin — named once because the "more tools"
/// panel is positioned from it: it drops down from the strip's outer edge, not
/// from the button that opened it.
pub(crate) const RIBBON_MARGIN_X: i8 = 14;
pub(crate) const RIBBON_MARGIN_Y: i8 = 8;

/// How wide the "more tools" button is. Narrower than a tool, because it holds
/// one arrow and no name — but as tall as one, so it sits in the row as part of
/// it.
const MORE_TOOLS_WIDTH: f32 = 26.0;

/// A small solid arrowhead, drawn as a shape rather than set as a character.
///
/// **Reported from use, twice, with a screenshot each time: "the glyph is not
/// an arrow, it's just a square".** `▾` and `⌃`/`⌄` are in none of the fonts this
/// program ships, so a character in their place is drawn as an empty box — the
/// silent failure `every_ribbon_glyph_can_actually_be_drawn` exists for, on two
/// controls that are not in the ribbon and so were never covered by it. A shape
/// needs nothing from a font and cannot come up as a box.
fn paint_arrow(painter: &egui::Painter, centre: egui::Pos2, up: bool, colour: egui::Color32) {
    let (half_width, half_height) = (6.0, 3.5);
    // A point's y grows downward, so the apex of a down-pointing arrow is the
    // lower of the three points.
    let apex = if up { -half_height } else { half_height };
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(centre.x - half_width, centre.y - apex),
            egui::pos2(centre.x + half_width, centre.y - apex),
            egui::pos2(centre.x, centre.y + apex),
        ],
        colour,
        egui::Stroke::NONE,
    ));
}

/// The arrow beside the command input that opens the history above it and
/// folds it away again.
///
/// **Reported from use, with a screenshot: its character was an empty square,
/// and "there should be an arrow for the user to open it".** Pointing up while
/// the history is shut, because that is the way it opens; down once it is open,
/// because that is the way it folds. One control for both directions rather
/// than an arrow to open and a cross to close — the second thing asked for.
pub(crate) fn history_toggle(ui: &mut egui::Ui, size: egui::Vec2, open: bool) -> egui::Response {
    let label = if open { "Hide the history" } else { "Show the history" };
    // An empty button for the frame and the hover state, with the arrow drawn
    // over it — the same chrome the character used to sit in.
    let response = ui.add_sized(size, egui::Button::new(""));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(response.rect) {
        let colour = ui.style().interact(&response).fg_stroke.color;
        paint_arrow(ui.painter(), response.rect.center(), !open, colour);
    }
    response.on_hover_text(label)
}

/// The button that drops the rest of a ribbon tab down.
///
/// **Reported from use, with a screenshot of the first version of it — a plain
/// text list: "instead of a drop down like this, just drop the rest of the
/// ribbon like how we had it."** What it opens is therefore the ribbon's own
/// tiles ([`tool_button`]), not a menu; this is only the handle.
pub(crate) fn more_tools_button(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(MORE_TOOLS_WIDTH, TOOL_HEIGHT), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "More tools")
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        // The same two states a tool has: framed while it is in force — here,
        // while the panel it opened is showing — or while the pointer is on it.
        if open {
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                theme::violet_deep(),
                egui::Stroke::new(1.0, theme::violet_bright()),
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
        // Pointing down to say "more below"; up once it is open, to say it will
        // fold back.
        paint_arrow(
            ui.painter(),
            rect.center(),
            open,
            if open || response.hovered() { theme::ink() } else { theme::ink_dim() },
        );
    }
    response.on_hover_text("More tools")
}

/// How many of a ribbon tab's own buttons (`widths`, one slot per button —
/// `TOOL_WIDTH` plus a divider's own width where one precedes it) fit in
/// `available_width` before the overflow menu's own room (`dropdown_reserve`)
/// is the only thing left. The index returned is where the "more tools" menu
/// takes over — **requested from use, with a screenshot**: a tab with more
/// tools than one row's width used to wrap them onto a permanent second row;
/// now everything from this index on is offered through a dropdown instead,
/// so the ribbon is always exactly one row tall. Pure, so the arithmetic
/// itself is checked without a running `Ui` — `ribbon_overflow_tests` below.
pub(crate) fn ribbon_overflow_at(available_width: f32, widths: &[f32], dropdown_reserve: f32) -> usize {
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        if available_width - used < w + dropdown_reserve {
            return i;
        }
        used += w;
    }
    widths.len()
}

/// A ribbon tab. Drawn rather than using `selectable_label`, so the chosen one
/// is the solid violet pill the mockup shows rather than a tinted background.
pub(crate) fn tab_button(ui: &mut egui::Ui, label: &str, chosen: bool) -> bool {
    let padding = egui::vec2(14.0, 7.0);
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(14.0),
        theme::ink(),
    );
    let size = galley.size() + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    let painter = ui.painter();
    if chosen {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::violet());
    } else if response.hovered() {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::raised());
    }
    let colour = if chosen {
        egui::Color32::WHITE
    } else if response.hovered() {
        theme::ink()
    } else {
        theme::ink_dim()
    };
    painter.galley(rect.min + padding, galley, colour);

    response.clicked()
}

/// What a document tab is made from — named once because the title bar sizes
/// its whole row from the same numbers (see [`doc_tab_height`]).
pub(crate) const DOC_TAB_FONT: f32 = 13.0;
const DOC_TAB_CLOSE: f32 = 14.0;
pub(crate) const DOC_TAB_PADDING: (f32, f32) = (12.0, 7.0);

/// How tall a document tab is — worked out by the same arithmetic
/// [`doc_tab_button`] sizes one with, so the two cannot drift apart.
///
/// **Needed by the title bar to put everything in its row on one line.** egui
/// centres each widget in the row *as tall as the row is when that widget is
/// placed*, and a row starts only a button high. The logo, the name and the
/// checkboxes were placed first, against that short row, and the tabs — taller
/// than any of them, and placed last — then extended it downward: measured, the
/// tabs' centre was 34.5 against 29.0 for everything else. Naming the height up
/// front gives every one of them the same centre.
pub(crate) fn doc_tab_height(ui: &egui::Ui) -> f32 {
    let line = ui
        .painter()
        .layout_no_wrap("M".to_string(), egui::FontId::proportional(DOC_TAB_FONT), egui::Color32::WHITE)
        .size()
        .y;
    line.max(DOC_TAB_CLOSE) + DOC_TAB_PADDING.1 * 2.0
}

/// One entry in the open-document strip — styled like `tab_button` above,
/// with a small "×" sharing the same clickable pill so there is no separate
/// hidden hit-box to miss.
///
/// `lifted` is a tab being carried somewhere else: it stays where it was, faded,
/// so the strip does not rearrange itself under the pointer — a copy of it is
/// what follows the pointer (see `hub`).
pub(crate) fn doc_tab_button(ui: &mut egui::Ui, label: &str, chosen: bool, lifted: bool) -> hub::TabButton {
    let padding = egui::vec2(DOC_TAB_PADDING.0, DOC_TAB_PADDING.1);
    let gap = 8.0;
    let close_size = DOC_TAB_CLOSE;
    let shown = fit_tab_label(ui, label, DOC_TAB_MAX_TEXT);
    let galley = ui.painter().layout_no_wrap(
        shown.clone(),
        egui::FontId::proportional(DOC_TAB_FONT),
        theme::ink(),
    );
    let size = egui::vec2(
        galley.size().x + padding.x * 2.0 + gap + close_size,
        (galley.size().y.max(close_size)) + padding.y * 2.0,
    );
    // Dragged as well as clicked: a tab can be carried to another window.
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    // Named for the screen reader — and for a test that has to find where on
    // screen this landed, which a bare painted rectangle gives it no way to do.
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, chosen, label));
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - padding.x - close_size / 2.0, rect.center().y),
        egui::Vec2::splat(close_size),
    );
    let over_close = ui.ctx().pointer_hover_pos().is_some_and(|p| close_rect.contains(p));

    let painter = ui.painter();
    if lifted {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::raised());
    } else if chosen {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::violet());
    } else if response.hovered() {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::raised());
    }
    let text_colour = if lifted {
        theme::ink_faint()
    } else if chosen {
        egui::Color32::WHITE
    } else if response.hovered() {
        theme::ink()
    } else {
        theme::ink_dim()
    };
    painter.galley(rect.min + padding, galley, text_colour);

    let close_colour = if lifted {
        theme::ink_faint()
    } else if response.hovered() && over_close {
        if chosen { egui::Color32::WHITE } else { theme::ink() }
    } else if chosen {
        egui::Color32::from_white_alpha(180)
    } else {
        theme::ink_dim()
    };
    painter.text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(close_size),
        close_colour,
    );

    let clicked = response.clicked();
    // A name that was cut says what it was when the pointer rests on it.
    let response = if shown != label { response.on_hover_text(label) } else { response };
    hub::TabButton {
        select: clicked && !over_close,
        close: clicked && over_close,
        close_rect,
        response,
    }
}

/// The longest a tab's name is drawn, in points. Longer names are cut in the
/// middle: documents that belong together are often told apart only by the end
/// of the name ("…-REV-02.pdf"), so that is what is kept.
pub(crate) const DOC_TAB_MAX_TEXT: f32 = 230.0;

/// The room kept for the triangle that opens the tabs that do not fit.
pub(crate) const DOC_TAB_MENU_WIDTH: f32 = 26.0;

/// `label`, or as much of it as fits `max` points with an ellipsis where the
/// middle was taken out, keeping the last characters.
pub(crate) fn fit_tab_label(ui: &egui::Ui, label: &str, max: f32) -> String {
    let width = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(DOC_TAB_FONT), egui::Color32::WHITE)
            .size()
            .x
    };
    if width(label) <= max {
        return label.to_string();
    }
    let chars: Vec<char> = label.chars().collect();
    let keep_tail = chars.len().min(14);
    let tail: String = chars[chars.len() - keep_tail..].iter().collect();
    for head in (0..=chars.len() - keep_tail).rev() {
        let candidate = format!("{}\u{2026}{tail}", chars[..head].iter().collect::<String>());
        if width(&candidate) <= max {
            return candidate;
        }
    }
    format!("\u{2026}{tail}")
}

/// How wide the tab for `label` is drawn, close button and padding included.
pub(crate) fn doc_tab_width(ui: &egui::Ui, label: &str) -> f32 {
    let text = fit_tab_label(ui, label, DOC_TAB_MAX_TEXT);
    let galley = ui
        .painter()
        .layout_no_wrap(text, egui::FontId::proportional(DOC_TAB_FONT), egui::Color32::WHITE);
    galley.size().x + DOC_TAB_PADDING.0 * 2.0 + 8.0 + DOC_TAB_CLOSE
}

/// How many of the tabs, taken from the left, fit in `room`. All of them when
/// they all do; otherwise as many as fit beside the menu button, and never
/// fewer than one — the strip is never empty.
pub(crate) fn tabs_that_fit(widths: &[f32], gap: f32, room: f32, menu: f32) -> usize {
    let count_within = |room: f32| {
        let mut used = 0.0;
        let mut n = 0;
        for &w in widths {
            let next = used + w + if n > 0 { gap } else { 0.0 };
            if next > room {
                break;
            }
            used = next;
            n += 1;
        }
        n
    };
    let all = count_within(room);
    if all == widths.len() {
        return all;
    }
    count_within(room - menu - gap).max(1)
}

/// The small triangle that opens the tabs the strip had no room for. Drawn as a
/// shape — a glyph for it may not exist in the font — with the count on hover.
pub(crate) fn tab_menu_button(ui: &mut egui::Ui, hidden: usize) -> egui::Response {
    let size = egui::vec2(DOC_TAB_MENU_WIDTH - 4.0, doc_tab_height(ui));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "More tabs"));
    if response.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, theme::raised());
    }
    let colour = if response.hovered() { theme::ink() } else { theme::ink_dim() };
    let c = rect.center();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![c + egui::vec2(-4.5, -2.5), c + egui::vec2(4.5, -2.5), c + egui::vec2(0.0, 3.5)],
        colour,
        egui::Stroke::NONE,
    ));
    response.on_hover_text(format!("{hidden} more open document{}", if hidden == 1 { "" } else { "s" }))
}
