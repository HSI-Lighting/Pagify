//! **Align to page**, from the Properties panel: put what is selected against an edge of the
//! page, or in the middle of it.
//!
//! Asked for by the owner: "align text or object with respect to the page, i.e. middle of the
//! page from the side margins, middle of the page from top margin etc." The page's own edges are
//! the margins (their choice), and the rule is `pagify_shell::align_page::delta`.
//!
//! **Aligned by what is seen.** Words are aligned by their ink (`Characters::ink_in`), not by
//! their text object's box, which can run far past the letters; centring the box would centre
//! the padding. A placed picture turned about its middle is aligned by its turned outline.
//!
//! **Moves go the way a drag's do.** An object, a placed picture or signature ends in the same
//! `finish_*` an arrow-key move ends in (`nudge.rs`); a group is one `MoveObjects` per page, so
//! one undo; markup shapes move through the markup layer's own move.

use crate::{Grab, PagifyApp, Selected};
use pagify_shell::align_page::{delta, AlignTo};
use pagify_shell::guides::Guides;
use pagify_shell::page_space::AppPoint;
use pdf_core::document::{Point, Rect};

/// What the Align to page buttons would move now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Aligned {
    /// The Edit Object tool's one thing.
    Object,
    /// The Edit Object tool's several.
    Group,
    /// A placed signature.
    Signature,
    /// A placed picture.
    Picture,
    /// Drawn markup shapes on this page.
    Shapes(usize),
    /// The open Edit Text paragraph.
    Paragraph,
    /// A new text box, before Add to Page.
    NewBox,
    /// Edit Object is in hand and nothing is picked: the panel is up so that the page does not
    /// move when something is.
    Waiting,
}

/// A move under this is "already there".
const ALREADY_PT: f32 = 0.01;
/// A placed picture or signature ignores a move under half a point (a drag's own jitter guard).
const IMAGE_FLOOR_PT: f32 = 0.5;
/// `finish_grab` and `finish_group_grab` drop a drag shorter than a few *screen* pixels as a wobbled
/// click. An alignment is not a drag: at this scale that is a few thousandths of a point.
const ALIGN_SCALE: f32 = 1.0e4;

impl PagifyApp {
    /// What the buttons would move: the same order the Properties panel picks its section in.
    pub(crate) fn align_subject(&self) -> Option<Aligned> {
        let tab = self.tab();
        let sel = &tab.selection;
        let object_tool = tab.tool_state.object_tool.is_some();
        let page = tab.view_state.page;
        if tab.edit.editing_run.is_some() {
            Some(Aligned::Paragraph)
        } else if tab.edit.new_text_box.is_some() {
            Some(Aligned::NewBox)
        } else if tab.markup.existing(page).is_some_and(|l| !l.selection().is_empty()) {
            Some(Aligned::Shapes(page))
        } else if object_tool && !sel.group.is_empty() {
            Some(Aligned::Group)
        } else if object_tool && sel.selected.is_some() {
            Some(Aligned::Object)
        } else if sel.signature_selected.is_some() {
            Some(Aligned::Signature)
        } else if sel.placed_image_selected.is_some() {
            Some(Aligned::Picture)
        } else if object_tool {
            Some(Aligned::Waiting)
        } else {
            None
        }
    }

    /// What the Properties panel says is selected, for the kinds that have no section of their own.
    pub(crate) fn align_describe(&self, subject: Aligned) -> String {
        match subject {
            Aligned::Group => format!("{} things selected", self.tab().selection.group.len()),
            Aligned::Object => match self.tab().selection.selected.as_ref().map(|s| s.what) {
                Some(what) => format!("Selected: {what}"),
                None => "Selected".into(),
            },
            Aligned::Signature => "A signature is selected".into(),
            Aligned::Picture => "A picture is selected".into(),
            Aligned::Waiting => "Pick something to align it to the page.".into(),
            _ => String::new(),
        }
    }

    /// Why the buttons are greyed, if they are. Only a paragraph can be: it moves as it is on the
    /// page, so words not applied yet are not on it, and a line drawn as shapes has no text object
    /// to move with the rest.
    pub(crate) fn align_refusal(&self, subject: Aligned) -> Option<String> {
        if subject == Aligned::Waiting {
            return Some("Pick something first.".into());
        }
        if subject != Aligned::Paragraph {
            return None;
        }
        let e = self.tab().edit.editing_run.as_ref()?;
        if e.changed() {
            return Some("Apply or cancel the edit first: the changed words are not on the page yet.".into());
        }
        let drawn = e.lines.iter().enumerate().any(|(i, (objects, _))| objects.is_empty() || e.frozen.get(i).copied().unwrap_or(false));
        drawn.then(|| "Part of this paragraph is drawn as shapes, which would stay behind.".into())
    }

    /// The Properties panel's section: six buttons, across then down.
    pub(crate) fn draw_align_to_page(&mut self, ui: &mut egui::Ui) {
        let Some(subject) = self.align_subject() else { return };
        let refusal = self.align_refusal(subject);
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Align to page").strong());
        ui.add_space(2.0);
        let mut chosen = None;
        for row in AlignTo::ALL.chunks(3) {
            ui.horizontal(|ui| {
                for &how in row {
                    let button = ui.add_enabled(refusal.is_none(), egui::Button::new(how.label())).on_hover_text(how.hint());
                    let button = match &refusal {
                        Some(why) => button.on_disabled_hover_text(why),
                        None => button,
                    };
                    if button.clicked() {
                        chosen = Some(how);
                    }
                }
            });
        }
        if let Some(how) = chosen {
            self.align_to_page(how);
        }
    }

    /// Put what is selected where `how` says, and say what happened.
    pub(crate) fn align_to_page(&mut self, how: AlignTo) {
        let Some(subject) = self.align_subject() else { return };
        let told = match self.align_refusal(subject) {
            Some(why) => Err(why),
            None => self.align_subject_to(subject, how),
        };
        match told {
            Ok(Some(said)) => self.say_info(said),
            Ok(None) => {}
            Err(e) => self.say_error(e),
        }
    }

    fn page_size_of(&self, page: usize) -> Result<(f32, f32), String> {
        self.tab().doc.as_ref().and_then(|d| d.strip.size_of(page)).ok_or_else(|| "that page has no size to align to.".to_string())
    }

    /// What a selected thing looks like on the page: the ink of words, else its own box.
    fn shown_rect(&mut self, page: usize, sel: &Selected) -> Rect {
        let r = sel.rect;
        if sel.is_text() {
            if let Some(ink) = self.characters(page).and_then(|c| c.ink_in((r.left, r.top, r.right, r.bottom))) {
                return ink;
            }
        }
        r
    }

    fn aligned(how: AlignTo, what: &str) -> String {
        format!("Aligned {what} to the page — {}.", how.label().to_lowercase())
    }

    /// `Ok(None)`: it said what it did itself, as a drag does.
    fn align_subject_to(&mut self, subject: Aligned, how: AlignTo) -> Result<Option<String>, String> {
        let nothing = || "nothing is selected.".to_string();
        let already = || Ok(Some("Already there.".to_string()));
        let by_hand = |page_size: (f32, f32), shown: Rect, floor: f32| {
            let (dx, dy) = delta(shown, page_size, how);
            (dx.abs() >= floor || dy.abs() >= floor).then_some((dx, dy))
        };
        let drag = |by: (f32, f32)| Grab { handle: None, from: AppPoint::new(0.0, 0.0), by };
        match subject {
            Aligned::Object => {
                let sel = self.tab().selection.selected.clone().ok_or_else(nothing)?;
                let size = self.page_size_of(sel.page)?;
                let shown = self.shown_rect(sel.page, &sel);
                let Some(by) = by_hand(size, shown, ALREADY_PT) else { return already() };
                self.finish_grab(sel, drag(by), ALIGN_SCALE, false);
                Ok(None)
            }
            Aligned::Signature => {
                let sel = self.tab().selection.signature_selected.clone().ok_or_else(nothing)?;
                let size = self.page_size_of(sel.page)?;
                let Some(by) = by_hand(size, turned_outline(sel.rect, sel.rotation), IMAGE_FLOOR_PT) else { return already() };
                self.finish_signature_grab(sel, drag(by), false);
                Ok(Some(Self::aligned(how, "the signature")))
            }
            Aligned::Picture => {
                let sel = self.tab().selection.placed_image_selected.clone().ok_or_else(nothing)?;
                let size = self.page_size_of(sel.page)?;
                let Some(by) = by_hand(size, turned_outline(sel.rect, sel.rotation), IMAGE_FLOOR_PT) else { return already() };
                self.finish_placed_image_grab(sel, drag(by), false);
                Ok(Some(Self::aligned(how, "the picture")))
            }
            Aligned::Group => self.align_group(how),
            Aligned::Shapes(page) => {
                let bounds = self.markup_selection_bounds(page).ok_or_else(nothing)?;
                let size = self.page_size_of(page)?;
                let Some((dx, dy)) = by_hand(size, bounds, ALREADY_PT) else { return already() };
                self.move_markup_by(page, dx, dy);
                Ok(None)
            }
            Aligned::Paragraph => self.align_paragraph(how),
            Aligned::Waiting => Err(nothing()),
            Aligned::NewBox => {
                let (page, rect) = self.tab().edit.new_text_box.as_ref().map(|b| (b.page, b.rect)).ok_or_else(nothing)?;
                let size = self.page_size_of(page)?;
                let Some((dx, dy)) = by_hand(size, rect, ALREADY_PT) else { return already() };
                if let Some(b) = self.tab_mut().edit.new_text_box.as_mut() {
                    b.rect = Rect { left: rect.left + dx, top: rect.top + dy, right: rect.right + dx, bottom: rect.bottom + dy };
                }
                Ok(Some(Self::aligned(how, "the text box")))
            }
        }
    }

    /// Every member of the group, **by the box round all of them on its page**, so the group keeps
    /// the members' places relative to each other. One `MoveObjects` per page, in one `Batch`: one undo.
    fn align_group(&mut self, how: AlignTo) -> Result<Option<String>, String> {
        let members = self.tab().selection.group.clone();
        let mut pages: std::collections::BTreeMap<usize, (Rect, Vec<usize>)> = std::collections::BTreeMap::new();
        for m in &members {
            let shown = self.shown_rect(m.page, m);
            let (bounds, objects) = pages.entry(m.page).or_insert((shown, Vec::new()));
            bounds.left = bounds.left.min(shown.left);
            bounds.top = bounds.top.min(shown.top);
            bounds.right = bounds.right.max(shown.right);
            bounds.bottom = bounds.bottom.max(shown.bottom);
            objects.push(m.object);
        }
        let mut shifts = std::collections::BTreeMap::new();
        let mut commands = Vec::new();
        for (page, (bounds, objects)) in pages {
            let (dx, dy) = delta(bounds, self.page_size_of(page)?, how);
            if dx.abs() < ALREADY_PT && dy.abs() < ALREADY_PT {
                continue;
            }
            shifts.insert(page, (dx, dy));
            commands.push(pdf_core::command::Command::MoveObjects { page_index: page, objects, by: Point { x: dx, y: dy } });
        }
        if commands.is_empty() {
            return Ok(Some("Already there.".into()));
        }
        self.run_group_command(commands).map_err(|e| format!("nothing was moved; {e}"))?;
        for m in &mut self.tab_mut().selection.group {
            if let Some(&(dx, dy)) = shifts.get(&m.page) {
                m.rect = Rect { left: m.rect.left + dx, top: m.rect.top + dy, right: m.rect.right + dx, bottom: m.rect.bottom + dy };
            }
        }
        self.forget_layers();
        Ok(Some(format!("{} — one `undo` puts them back.", Self::aligned(how, &format!("{} things", members.len())))))
    }

    /// The open paragraph's lines move together, by the box round their ink; the box is then
    /// opened again over them where they landed. The words are not touched (a move keeps their
    /// codes and spacing); a paragraph with unapplied changes is refused before this (`align_refusal`).
    fn align_paragraph(&mut self, how: AlignTo) -> Result<Option<String>, String> {
        let (page, rect, objects) = {
            let e = self.tab().edit.editing_run.as_ref().ok_or_else(|| "nothing is open.".to_string())?;
            let objects: Vec<usize> =
                e.lines.iter().flat_map(|(o, _)| o.iter().copied()).chain(e.twins.iter().flatten().copied()).collect();
            (e.page, e.rect, objects)
        };
        let size = self.page_size_of(page)?;
        let shown = self.characters(page).and_then(|c| c.ink_in((rect.left, rect.top, rect.right, rect.bottom))).unwrap_or(rect);
        let (dx, dy) = delta(shown, size, how);
        if dx.abs() < ALREADY_PT && dy.abs() < ALREADY_PT {
            return Ok(Some("Already there.".into()));
        }
        let by = Point { x: dx, y: dy };
        self.run_group_command(vec![pdf_core::command::Command::MoveObjects { page_index: page, objects, by }])
            .map_err(|e| format!("nothing was moved; {e}"))?;
        // Put the box down and open it again where the words are now, in this same call, so no
        // frame sees an editor whose page changed under it.
        self.put_down_page_editors("aligned to the page");
        let centre = AppPoint::new(((rect.left + rect.right) / 2.0 + dx) as f64, ((rect.top + rect.bottom) / 2.0 + dy) as f64);
        let said = Self::aligned(how, "the paragraph");
        Ok(Some(match self.pick_text_run(page, centre) {
            Ok(_) => said,
            Err(e) => format!("{said} It could not be opened again: {e}"),
        }))
    }
}

/// Which drag a picture, signature or drawn shape is in.
#[derive(Clone, Copy)]
enum Dragged {
    Signature,
    Picture,
    Markup,
}

impl PagifyApp {
    /// A signature, a placed picture or a drawn shape being dragged by its body on `page`: what
    /// it covers before the drag, and how far the drag has come.
    fn dragged_by_body(&self, page: usize) -> Option<(Dragged, Rect, (f32, f32))> {
        let sel = &self.tab().selection;
        if let (Some(g), Some(s)) = (&sel.signature_grab, &sel.signature_selected) {
            if g.handle.is_none() && s.page == page {
                return Some((Dragged::Signature, turned_outline(s.rect, s.rotation), g.by));
            }
        }
        if let (Some(g), Some(s)) = (&sel.placed_image_grab, &sel.placed_image_selected) {
            if g.handle.is_none() && s.page == page {
                return Some((Dragged::Picture, turned_outline(s.rect, s.rotation), g.by));
            }
        }
        let g = sel.markup_grab.as_ref().filter(|g| g.handle.is_none())?;
        Some((Dragged::Markup, self.markup_selection_bounds(page)?, g.by))
    }

    /// **Pull a picture, signature or drawn shape onto the page's own edges and middles** while it
    /// is dragged, as Edit Object's things are (`snap_the_move`). Only the page's lines, not other
    /// things': these are not page content. `snap` is off while Alt is held; `ortho` (Shift) keeps
    /// the pull on the axis the drag is held to.
    pub(crate) fn snap_to_page_lines(&mut self, page: usize, scale: f32, snap: bool, ortho: bool) {
        let Some((which, rect, by)) = self.dragged_by_body(page) else { return };
        let Ok(size) = self.page_size_of(page) else { return };
        let reach = if snap { Self::GUIDE_SNAP_PX / scale.max(0.01) } else { 0.0 };
        let (nudge, _) = pagify_shell::guides::align(Self::shifted(rect, by), &[], size, reach);
        let nudge = if ortho { Self::along_the_held_axis(by, nudge) } else { nudge };
        let sel = &mut self.tab_mut().selection;
        let slot = match which {
            Dragged::Signature => &mut sel.signature_grab,
            Dragged::Picture => &mut sel.placed_image_grab,
            Dragged::Markup => &mut sel.markup_grab,
        };
        if let Some(g) = slot.as_mut() {
            g.by = (g.by.0 + nudge.0, g.by.1 + nudge.1);
        }
    }

    /// The reference lines to draw for such a drag: its own sides and middle, green where it sits
    /// exactly on the page's edge or middle.
    pub(crate) fn page_lines_while_dragging(&self, page: usize) -> Option<Guides> {
        let (_, rect, by) = self.dragged_by_body(page)?;
        let size = self.page_size_of(page).ok()?;
        Some(pagify_shell::guides::align(Self::shifted(rect, by), &[], size, 0.0).1)
    }
}

/// The smallest upright rectangle round `rect` turned `degrees` about its own middle: what a turned
/// picture shows.
pub(crate) fn turned_outline(rect: Rect, degrees: f32) -> Rect {
    let (w, h) = ((rect.right - rect.left).abs(), (rect.bottom - rect.top).abs());
    let (cx, cy) = ((rect.left + rect.right) / 2.0, (rect.top + rect.bottom) / 2.0);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (half_w, half_h) = ((w * cos.abs() + h * sin.abs()) / 2.0, (w * sin.abs() + h * cos.abs()) / 2.0);
    Rect { left: cx - half_w, top: cy - half_h, right: cx + half_w, bottom: cy + half_h }
}
