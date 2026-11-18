//! **Rotation in the Properties panel** (owner's request, 2026-11-18): "in properties add rotation
//! angles".
//!
//! An angle box showing the current angle and taking a typed one, plus **Left 90°**, **Right 90°**
//! and **Reset**, for the three kinds of picture:
//!
//! - a placed or pasted picture ([`PlacedImageSelected`]) and a signature picture
//!   ([`SignatureSelected`]) turn to an **absolute** angle, clockwise as seen — the same call the ring
//!   drag makes;
//! - a picture that is part of the page (what a pasted picture becomes after Save and reopen) turns
//!   **by** the shortest difference to the angle asked for, about its own centre, through
//!   `rotate_thing` — the ring drag's own path.
//!
//! **Nothing here turns per frame.** A placed picture's turn rebuilds its appearance
//! (`redraw_picture`), so the typed angle is applied only when the edit ends: the drag released, Enter,
//! or the box losing focus. Each change is one command, so one undo.
//!
//! **Words, shapes, groups and paragraphs are not offered.** They have no angle of their own to show,
//! and a rotation section that did nothing would be worse than none.

use crate::align_page::Aligned;
use crate::{PagifyApp, PlacedImageSelected, SignatureSelected};
use pagify_shell::page_space::AppPoint;
use pdf_core::document::Point;

/// What the rotation controls would turn now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Turnable {
    /// A placed or pasted picture, picked with no tool in hand.
    Placed(PlacedImageSelected),
    /// A picture marked as a signature, picked the same way.
    Signature(SignatureSelected),
    /// A picture that is part of the page, picked with Edit Object.
    PagePicture {
        page: usize,
        object: usize,
        rect: pdf_core::document::Rect,
        /// The clockwise-as-seen angle its matrix gives, with the page's own turn included.
        angle: f32,
    },
}

/// Which picture a typed angle belongs to. Stored with the draft, so a click on another picture
/// before Enter drops the typing instead of turning the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RotationKey {
    Placed { page: usize, index: usize },
    Signature { page: usize, index: usize },
    PagePicture { page: usize, object: usize },
}

impl Turnable {
    fn key(&self) -> RotationKey {
        match self {
            Turnable::Placed(selected) => RotationKey::Placed { page: selected.page, index: selected.index },
            Turnable::Signature(selected) => {
                RotationKey::Signature { page: selected.page, index: selected.index }
            }
            Turnable::PagePicture { page, object, .. } => {
                RotationKey::PagePicture { page: *page, object: *object }
            }
        }
    }
}

/// An angle in `[0, 360)`, with `-0.0` coming back as `0.0`.
pub(crate) fn normalised(degrees: f32) -> f32 {
    if !degrees.is_finite() {
        return 0.0;
    }
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped == 0.0 || wrapped >= 360.0 {
        // Both comparisons match -0.0, and the literal is the positive zero.
        0.0
    } else {
        wrapped
    }
}

/// How far to turn from `from` to `to`, in `(-180, 180]` — the shortest way round.
pub(crate) fn shortest_turn(from: f32, to: f32) -> f32 {
    let mut delta = (to - from).rem_euclid(360.0);
    if delta > 180.0 {
        delta -= 360.0;
    }
    delta
}

/// The clockwise-as-seen angle of the image's x axis, from its PDF matrix, plus the page's own
/// clockwise quarter turns; rounded to 0.1°.
///
/// PDF space is y-up, so the matrix's angle is counter-clockwise and is negated to read as a clock
/// face. A page turned `/Rotate 90` is shown a quarter turn clockwise, which adds 90.
pub(crate) fn shown_angle(matrix: [f32; 6], quarter_turns: u8) -> f32 {
    let degrees = -(matrix[1].atan2(matrix[0])).to_degrees() + 90.0 * quarter_turns as f32;
    let rounded = (normalised(degrees) * 10.0).round() / 10.0;
    if rounded >= 360.0 {
        0.0
    } else {
        rounded
    }
}

impl PagifyApp {
    /// What the rotation controls would turn: the same subjects the Properties panel picks its
    /// section by ([`Self::align_subject`]), narrowed to the three kinds of picture.
    pub(crate) fn rotation_subject(&self) -> Option<Turnable> {
        match self.align_subject()? {
            Aligned::Picture => self
                .tab()
                .selection
                .placed_image_selected
                .clone()
                .map(Turnable::Placed),
            Aligned::Signature => self
                .tab()
                .selection
                .signature_selected
                .clone()
                .map(Turnable::Signature),
            Aligned::Object => {
                let selected = self
                    .tab()
                    .selection
                    .selected
                    .clone()
                    .filter(|selected| selected.what == "the picture")?;
                let page = selected.page;
                let quarter_turns = self.tab().doc.as_ref()?.session.page_rotation(page).ok()?;
                let image = self.images_on(page).into_iter().find(|image| image.object == selected.object)?;
                Some(Turnable::PagePicture {
                    page,
                    object: selected.object,
                    rect: selected.rect,
                    angle: shown_angle(image.matrix, quarter_turns),
                })
            }
            _ => None,
        }
    }

    /// Turn what is selected to an absolute angle, clockwise as seen. One command, so one undo; a
    /// change smaller than a twentieth of a degree is not one at all.
    pub(crate) fn turn_selected_to(&mut self, degrees: f32) {
        let wanted = normalised(degrees);
        let Some(subject) = self.rotation_subject() else { return };
        let current = match &subject {
            Turnable::Placed(selected) => selected.rotation,
            Turnable::Signature(selected) => selected.rotation,
            Turnable::PagePicture { angle, .. } => *angle,
        };
        if shortest_turn(current, wanted).abs() < 0.05 {
            return;
        }
        match subject {
            Turnable::Placed(selected) => self.turn_placed_image_to(selected, wanted),
            Turnable::Signature(selected) => self.turn_signature_to(selected, wanted),
            Turnable::PagePicture { page, object, rect, angle } => {
                let pivot = Point {
                    x: (rect.left + rect.right) / 2.0,
                    y: (rect.top + rect.bottom) / 2.0,
                };
                let told = self.rotate_thing(page, object, pivot, shortest_turn(angle, wanted));
                match told {
                    Ok(said) => self.say_info(said),
                    Err(e) => self.say_error(e),
                }
                // Where it is now, the way the ring drag's own path finds it again: the middle does
                // not move under a turn, and the page has been rewritten so the object numbers may
                // have.
                let middle = AppPoint::new(pivot.x as f64, pivot.y as f64);
                self.forget_layers();
                if !self.select_thing_at_drilling(page, middle, false) {
                    self.tab_mut().selection.selected = None;
                }
            }
        }
    }

    /// The three buttons: turn by `delta` from where it is now.
    pub(crate) fn turn_selected_by(&mut self, delta: f32) {
        let Some(subject) = self.rotation_subject() else { return };
        let current = match &subject {
            Turnable::Placed(selected) => selected.rotation,
            Turnable::Signature(selected) => selected.rotation,
            Turnable::PagePicture { angle, .. } => *angle,
        };
        self.turn_selected_to(current + delta);
    }

    /// The Properties panel's rotation section, for the three kinds of picture and nothing else.
    pub(crate) fn draw_rotation(&mut self, ui: &mut egui::Ui) {
        let Some(subject) = self.rotation_subject() else { return };
        let current = match &subject {
            Turnable::Placed(selected) => selected.rotation,
            Turnable::Signature(selected) => selected.rotation,
            Turnable::PagePicture { angle, .. } => *angle,
        };

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Rotation").strong());
        ui.add_space(2.0);

        // The typed number, applied only when the edit ends. A DragValue applies on every frame it
        // is dragged; a placed picture's every turn rebuilds its appearance, and a page picture's is
        // a page rewrite, so per-frame would be dozens of undos for one gesture.
        let key = subject.key();
        let mut typed: Option<f32> = None;
        let draft = self
            .tab()
            .panels
            .rotation_draft
            .filter(|(draft_key, _)| *draft_key == key)
            .map(|(_, value)| value)
            .unwrap_or(current);
        ui.horizontal(|ui| {
            ui.label("Angle");
            let mut value = draft;
            // **A fresh widget per picture.** egui keeps a DragValue's half-typed text in memory
            // by widget id; with one id for every subject, the text typed for one picture would be
            // shown — and parsed — for the next one after a click. The key in the id makes the old
            // box disappear when the subject changes, and with it the stale text.
            let response = ui
                .push_id(key, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut value)
                            .range(0.0..=359.9)
                            .speed(1.0)
                            .max_decimals(1)
                            .suffix("°"),
                    )
                })
                .inner;
            if response.changed() {
                self.tab_mut().panels.rotation_draft = Some((key, value));
            }
            if response.changed() || response.has_focus() {
                self.tab_mut().panels.rotation_key = Some(key);
            }
            if response.drag_stopped() || response.lost_focus() {
                typed = Some(
                    self.tab()
                        .panels
                        .rotation_draft
                        .filter(|(draft_key, _)| *draft_key == key)
                        .map(|(_, drafted)| drafted)
                        .unwrap_or(value),
                );
            }
        });

        let mut by: Option<f32> = None;
        let mut reset = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Left 90°").on_hover_text("Turn 90° anticlockwise").clicked() {
                by = Some(-90.0);
            }
            if ui.button("Right 90°").on_hover_text("Turn 90° clockwise").clicked() {
                by = Some(90.0);
            }
            if ui.button("Reset").on_hover_text("Back to 0°").clicked() {
                reset = true;
            }
        });

        if let Some(value) = typed {
            let owner = self.tab_mut().panels.rotation_key.take();
            self.tab_mut().panels.rotation_draft = None;
            // **Only the picture it was typed for.** egui reports `lost_focus` on the frame after
            // the click that took it away, and by then the click may have selected another picture;
            // a half-typed angle must not land on that one. The box records whose it was while it
            // held the focus, which is what catches a value typed without a change event.
            if owner == Some(key) && self.rotation_subject().is_some_and(|subject| subject.key() == key) {
                self.turn_selected_to(value);
            }
        }
        if reset {
            self.tab_mut().panels.rotation_draft = None;
            self.tab_mut().panels.rotation_key = None;
            self.turn_selected_to(0.0);
        } else if let Some(delta) = by {
            self.tab_mut().panels.rotation_draft = None;
            self.tab_mut().panels.rotation_key = None;
            self.turn_selected_by(delta);
        }
    }
}
