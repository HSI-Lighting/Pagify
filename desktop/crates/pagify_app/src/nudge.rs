//! Moving the selection with the arrow keys.
//!
//! **Asked for:** move objects with the arrow keys, finer the further the page is
//! zoomed in. A press moves the selection [`PagifyApp::NUDGE_PX`] pixels *as seen*
//! — so a zoomed-in page moves it by less and a zoomed-out one by more — and Shift
//! makes it ten times that.
//!
//! **A run of presses is one move.** A held key repeats ~30 times a second, and a
//! move is a rewrite of the page (about 40–80 ms), with an undo step of its own.
//! So the presses add up in the selection's own drag — the same `Grab` a mouse
//! drag fills in, and so drawn, and shown with its reference lines, by the code
//! that already draws that drag — and land together, through the same `finish_*`
//! a mouse drag ends in, once the keys have been still for [`NUDGE_SETTLE`].
//! Anything else that acts on the document (a click, a key, undo, save) lands it
//! first, so it never acts on a selection that is not where it is drawn; Escape
//! drops it, as it drops a drag.

use crate::{Grab, PagifyApp};
use pagify_shell::page_space::AppPoint;

/// How long the arrow keys have to be still before the moves land, in seconds.
const NUDGE_SETTLE: f64 = 0.35;

/// What the arrow keys move: the selections that have a drag of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// The object tool's one selected thing.
    Object,
    /// The object tool's several.
    Group,
    /// A placed signature.
    Signature,
    /// A placed picture.
    Picture,
}

impl PagifyApp {
    /// How far one press moves the selection, in screen pixels. More than
    /// [`Self::MIN_DRAG_PX`], or the move would be taken for a click that wobbled
    /// and dropped.
    pub(crate) const NUDGE_PX: f32 = 4.0;
    /// The least a press moves a placed picture or signature, in page points: just over
    /// the half point `finish_placed_image_grab` / `finish_signature_grab` ignore.
    const NUDGE_MIN_PT: f32 = 0.55;
    /// What Shift multiplies a press by.
    const NUDGE_SHIFT_FACTOR: f32 = 10.0;

    /// What the arrow keys would move now, if anything.
    fn nudge_target(&self) -> Option<Held> {
        let sel = &self.tab().selection;
        let object_tool = self.tab().tool_state.object_tool.is_some();
        if object_tool && !sel.group.is_empty() {
            Some(Held::Group)
        } else if object_tool && sel.selected.is_some() {
            Some(Held::Object)
        } else if sel.signature_selected.is_some() {
            Some(Held::Signature)
        } else if sel.placed_image_selected.is_some() {
            Some(Held::Picture)
        } else {
            None
        }
    }

    fn nudge_slot(&mut self, held: Held) -> &mut Option<Grab> {
        let sel = &mut self.tab_mut().selection;
        match held {
            Held::Object => &mut sel.grab,
            Held::Group => &mut sel.group_grab,
            Held::Signature => &mut sel.signature_grab,
            Held::Picture => &mut sel.placed_image_grab,
        }
    }

    /// Screen pixels per page point: the logical zoom, in the pixels it draws at.
    pub(crate) fn nudge_scale(&self) -> f32 {
        (self.resolved_zoom() * Self::DISPLAY_DPI_SCALE).max(0.01)
    }

    /// Take this frame's arrow presses for the selection, if there is one to move.
    /// `true` means there is one, so the arrows are its and not the page turn's.
    pub(crate) fn nudge_selection(&mut self, ctx: &egui::Context) -> bool {
        let Some(held) = self.nudge_target() else { return false };
        let (across, down, shift) = ctx.input(|i| {
            let n = |k| i.num_presses(k) as f32;
            (
                n(egui::Key::ArrowRight) - n(egui::Key::ArrowLeft),
                n(egui::Key::ArrowDown) - n(egui::Key::ArrowUp),
                i.modifiers.shift,
            )
        });
        if across == 0.0 && down == 0.0 {
            return true;
        }
        let pending = self.tab().selection.nudge_at.is_some();
        // A drag with the mouse is under way: the keys wait for it to end.
        if !pending && self.nudge_slot(held).is_some() {
            return true;
        }
        let mut step = Self::NUDGE_PX / self.nudge_scale();
        // A placed picture or signature ignores a move under half a point (a drag's own
        // jitter guard), so at a high zoom a finer step would do nothing at all.
        if matches!(held, Held::Signature | Held::Picture) {
            step = step.max(Self::NUDGE_MIN_PT);
        }
        if shift {
            step *= Self::NUDGE_SHIFT_FACTOR;
        }
        let grab = self
            .nudge_slot(held)
            .get_or_insert(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (0.0, 0.0) });
        grab.by.0 += across * step;
        grab.by.1 += down * step;
        self.tab_mut().selection.nudge_at = Some(ctx.input(|i| i.time));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(NUDGE_SETTLE));
        true
    }

    /// Land the moves the arrow keys have built up, once they have settled — or
    /// now, when `now`. Called every frame, before anything else acts.
    pub(crate) fn settle_nudge(&mut self, ctx: &egui::Context, now: bool) {
        let Some(last) = self.tab().selection.nudge_at else { return };
        let waited = ctx.input(|i| i.time) - last;
        if !now && waited < NUDGE_SETTLE {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(NUDGE_SETTLE - waited));
            return;
        }
        self.tab_mut().selection.nudge_at = None;
        let scale = self.nudge_scale();
        // The same endings a mouse drag has (`Self::interact_objects` and the
        // two after it); only the one the keys filled in has a drag to end.
        if let (Some(grab), Some(sel)) = (self.tab_mut().selection.grab.take(), self.tab().selection.selected.clone()) {
            self.finish_grab(sel, grab, scale);
        }
        if let Some(grab) = self.tab_mut().selection.group_grab.take() {
            self.finish_group_grab(grab, scale);
        }
        if let (Some(grab), Some(sel)) =
            (self.tab_mut().selection.signature_grab.take(), self.tab().selection.signature_selected.clone())
        {
            self.finish_signature_grab(sel, grab);
        }
        if let (Some(grab), Some(sel)) =
            (self.tab_mut().selection.placed_image_grab.take(), self.tab().selection.placed_image_selected.clone())
        {
            self.finish_placed_image_grab(sel, grab);
        }
    }

    /// Drop the moves the arrow keys have built up, as Escape drops a drag.
    pub(crate) fn drop_nudge(&mut self) {
        if self.tab_mut().selection.nudge_at.take().is_some() {
            let sel = &mut self.tab_mut().selection;
            sel.grab = None;
            sel.group_grab = None;
            sel.signature_grab = None;
            sel.placed_image_grab = None;
        }
    }

    /// What this frame did that must see the selection where it is drawn: a
    /// press of the pointer, or any key but an arrow, or a copy, cut or paste.
    pub(crate) fn frame_acts_on_the_selection(ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.pointer.any_pressed()
                || i.events.iter().any(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => !matches!(
                        key,
                        egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown
                    ),
                    egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => true,
                    _ => false,
                })
        })
    }
}
