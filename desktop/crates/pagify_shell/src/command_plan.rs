//! What a command *does*, as data — the first slice of the design review's
//! Phase 4b (DESIGN_REVIEW.md §3.4).
//!
//! [`plan`] turns the document, view and app verbs into [`Effect`]s that the
//! app applies. Verbs the plan does not know return `None`, and the app's own
//! handlers still run them; the migration ports one domain at a time, and
//! this is the first — the eleven verbs of `dispatch::act_document`.
//!
//! Planning is deliberately pure: no engine, no window, no `egui`. That is
//! what makes the decision side testable in this crate, while the app keeps
//! ownership of *doing* things (`Effect` application lives in
//! `pagify_app::dispatch`).

use crate::page_space::{AppPoint, PageSpace};
use crate::verbs::{AppPointArg, MeasureKind, PageTarget, Verb, ZoomTarget};
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
}

impl Default for CommandEnv {
    fn default() -> Self {
        Self { has_doc: false, page: 0, page_height: 792.0, tool_armed: false }
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
}

/// The plan for a verb, or `None` when this slice does not know it yet — the
/// app then runs its own handler, so an unported verb behaves exactly as
/// before.
pub fn plan(verb: &Verb, env: &CommandEnv) -> Option<Vec<Effect>> {
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
        Verb::Pdfium => vec![Effect::Say(crate::pdfium::describe())],
        Verb::Version => vec![Effect::Say(format!("Pagify {}", crate::VERSION))],
        // The message comes first so it is on the bar while the check runs.
        Verb::CheckUpdate => vec![Effect::Say("checking for a newer build…".into()), Effect::CheckUpdate],
        // Objects and measurement: mirrors of the verbs, so the app applies
        // the same bodies it always ran.
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
        // Tools: the guards and the one coordinate flip live here now, where
        // they can be tested without a window.
        Verb::Pick(at) => {
            if !env.tool_armed {
                vec![Effect::SayError("nothing is waiting for a click.".into())]
            } else {
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

#[cfg(test)]
mod tests {
    use super::*;
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
        ];
        for (verb, want) in cases {
            assert_eq!(plan(&verb, &CommandEnv::default()), Some(want), "planning {verb:?}");
        }
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
        let with_doc = CommandEnv { has_doc: true, page: 2, page_height: 800.0, tool_armed: true };
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
    fn unported_verbs_have_no_plan_yet() {        // One verb from each domain still handled app-side; the list shrinks
        // as the migration ports them.
        for verb in [
            Verb::Pointer(crate::verbs::PointerMode::Select),
            Verb::SaveAs(std::path::PathBuf::from("a.pdf")),
            Verb::Find(String::new()),
            Verb::Undo,
        ] {
            assert!(plan(&verb, &CommandEnv::default()).is_none(), "{verb:?} should still fall through");
        }
    }
}
