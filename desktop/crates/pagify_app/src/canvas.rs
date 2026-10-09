//! The canvas: the page strip itself (`draw_pages`), everything drawn over a
//! page (selection outlines, lock badges, the armed-tool preview, move
//! guides), and the pointer handling for the object tool, a placed
//! signature, and a placed picture — the review's `canvas.rs`.

use crate::overlay::{self, PageView};
use crate::theme;
use crate::{
    icon_font, raster_scale, reveal_axis, short, view_height, Awaiting, Grab, Handle,
    OrganizeDrag, PlacedImageSelected, Reveal, SignatureSelected, Tab, Tool, ToolEffect, ZoomMode,
    GRID_GAP_PT, HANDLE_PX, ORGANIZE_GRID_PANEL, ROTATE_HANDLE_PX,
};
use pagify_shell::command::Kind;
use pagify_shell::markup::HIT_TOLERANCE_PT;
use pagify_shell::page_space::AppPoint;
use pagify_shell::reader::{prefetch_targets, STRIP_PAD_PX};
use pagify_shell::tools;
use pagify_shell::verbs::{PageTarget, Verb};

/// How one box over the page is drawn — see [`crate::PagifyApp::paragraph_boxes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxStyle {
    /// A paragraph a click would open.
    Plain,
    /// The one the pointer is over.
    Lit,
    /// Lines the page draws as shapes: nothing a click can open.
    Drawn,
}

impl crate::PagifyApp {
    /// The reference lines, **only while something is being moved** — not while it
    /// is resized, and not when it is merely selected. Grey for a line near it,
    /// green for one a side or the middle is exactly on.
    pub(crate) fn draw_move_guides(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let (moving, exclude): (pdf_core::document::Rect, Vec<usize>) = {
            let single = match (self.tab().grab.clone(), self.tab().selected.clone()) {
                (Some(grab), Some(sel)) if grab.handle.is_none() && sel.page == page => {
                    Some((Self::shifted(sel.rect, grab.by), vec![sel.object]))
                }
                _ => None,
            };
            let group = || match (self.tab().group_grab.clone(), self.group_bounds(page)) {
                (Some(grab), Some(bounds)) if grab.handle.is_none() => {
                    let members = self.tab().group.iter().filter(|m| m.page == page).map(|m| m.object).collect();
                    Some((Self::shifted(bounds, grab.by), members))
                }
                _ => None,
            };
            match single.or_else(group) {
                Some(found) => found,
                None => return,
            }
        };
        let (_, guides) = self.align_for(page, moving, &exclude, view.scale, false);
        if guides.is_empty() {
            return;
        }
        let Some((page_w, page_h)) = self.tab().doc.as_ref().and_then(|d| d.strip.size_of(page)) else { return };
        let painter = ui.painter();
        let grey = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(120, 130, 150, 190));
        let green = egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 230, 0));
        // Grey first, so a green line over the same place is the one that shows.
        for exact in [false, true] {
            let stroke = if exact { green } else { grey };
            for line in guides.vertical.iter().filter(|l| l.exact == exact) {
                let top = view.to_screen(AppPoint::new(line.at as f64, 0.0));
                let bottom = view.to_screen(AppPoint::new(line.at as f64, page_h as f64));
                painter.line_segment([top, bottom], stroke);
            }
            for line in guides.horizontal.iter().filter(|l| l.exact == exact) {
                let left = view.to_screen(AppPoint::new(0.0, line.at as f64));
                let right = view.to_screen(AppPoint::new(page_w as f64, line.at as f64));
                painter.line_segment([left, right], stroke);
            }
        }
    }

    /// Forget what the object tool has picked. A placed picture or signature
    /// is selected by the same pointer, and two selections at once means
    /// Delete, Copy and the handles disagree about which one they mean.
    fn drop_object_selection(&mut self) {
        self.tab_mut().selected = None;
        self.tab_mut().group = Vec::new();
    }

    /// The object tool's own pointer handling: select on click, move by
    /// dragging the body, resize by dragging a handle. The document changes
    /// once, when the pointer is let go.
    pub(crate) fn interact_objects(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        page: usize,
        at: AppPoint,
        view: PageView,
    ) {
        // Read before the hover block below can overwrite it — see
        // `Self::object_hover_handle`'s own doc for why a drag that has just
        // started must be classified against *that*, not against a fresh
        // hit-test at `at`. Mirrors `Self::interact_signatures`.
        let remembered_handle = self.tab_mut().object_hover_handle;

        // The cursor says what a press here would do.
        if self.tab_mut().grab.is_none() && self.tab_mut().group_grab.is_none() {
            if let Some(rect) = self.tab().selected.as_ref().filter(|s| s.page == page).map(|s| s.rect) {
                let handle = self.handle_at(at, view);
                self.tab_mut().object_hover_handle = handle;
                if let Some(handle) = handle {
                    ui.output_mut(|o| o.cursor_icon = handle.cursor());
                } else if Self::point_in_rect(at, &rect) {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                }
            } else if let Some(bounds) = self.group_bounds(page) {
                let handle = Self::handle_near(at, view, &bounds);
                self.tab_mut().object_hover_handle = handle;
                if let Some(handle) = handle {
                    ui.output_mut(|o| o.cursor_icon = handle.cursor());
                } else if Self::point_in_rect(at, &bounds) {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                }
            } else {
                self.tab_mut().object_hover_handle = None;
            }
        }

        if response.drag_started() {
            let on_handle = self.tab_mut()
                .selected
                .as_ref()
                .filter(|s| s.page == page)
                .and_then(|_| remembered_handle);
            let on_body = self.tab_mut().selected.as_ref().is_some_and(|s| {
                s.page == page
                    && at.x >= s.rect.left as f64
                    && at.x <= s.rect.right as f64
                    && at.y >= s.rect.top as f64
                    && at.y <= s.rect.bottom as f64
            });
            let group_bounds = self.group_bounds(page);
            // A group's handles read from `remembered_handle` too — it is
            // whichever of the two hover branches above last ran, and
            // `self.tab_mut().selected`/`self.tab_mut().group` are never both populated at
            // once, so it always means the right one.
            let on_group_handle = group_bounds.is_some().then(|| remembered_handle).flatten();
            let on_group_body = on_group_handle.is_none()
                && group_bounds.as_ref().is_some_and(|b| Self::point_in_rect(at, b));
            if on_group_handle.is_some() || on_group_body {
                self.tab_mut().group_grab = Some(Grab { handle: on_group_handle, from: at, by: (0.0, 0.0) });
            } else if on_handle.is_some() || on_body {
                // A drag on the current selection's own body or a handle:
                // move or resize it.
                self.tab_mut().group = Vec::new();
                // **A turn is measured from where the pointer went down, not from
                // where egui decided it was a drag** — a few pixels along already,
                // which at the handle's distance from the middle is several
                // degrees the shape would jump by the moment it began to turn.
                let from = match (on_handle, ui.input(|i| i.pointer.press_origin())) {
                    (Some(Handle::Rotate), Some(pressed)) => view.to_page(pressed),
                    _ => at,
                };
                self.tab_mut().grab = Some(Grab { handle: on_handle, from, by: (0.0, 0.0) });
            } else {
                // **Reported from use: dragging out a marquee across
                // several objects kept grabbing and moving whichever one
                // the press happened to land on first**, instead of
                // drawing the box — a drag used to select-and-move
                // whatever was under it in one gesture, the same as a
                // plain click. Selecting one thing is now only ever a
                // click that releases without moving (`response.clicked()`
                // below); a drag, wherever it starts, is always the
                // marquee. Moving something still works — select it with a
                // click first, then drag its own body or a handle, which
                // the branch above this one still covers exactly as
                // before.
                self.tab_mut().selected = None;
                // A picture or signature picked a moment ago is not part of
                // what this marquee is about to select.
                self.tab_mut().placed_image_selected = None;
                self.tab_mut().signature_selected = None;
                if !ui.input(|i| i.modifiers.shift) {
                    self.tab_mut().group = Vec::new();
                }
                self.tab_mut().marquee = Some((at, at));
            }
        }

        if response.dragged() {
            if let Some(grab) = self.tab_mut().grab.as_mut() {
                grab.by = ((at.x - grab.from.x) as f32, (at.y - grab.from.y) as f32);
                ui.output_mut(|o| {
                    o.cursor_icon = match grab.handle {
                        Some(h) => h.cursor(),
                        None => egui::CursorIcon::Grabbing,
                    }
                });
            }
            if let Some(grab) = self.tab_mut().group_grab.as_mut() {
                grab.by = ((at.x - grab.from.x) as f32, (at.y - grab.from.y) as f32);
                ui.output_mut(|o| {
                    o.cursor_icon = match grab.handle {
                        Some(h) => h.cursor(),
                        None => egui::CursorIcon::Grabbing,
                    }
                });
            }
            if let Some((_, current)) = self.tab_mut().marquee.as_mut() {
                *current = at;
            }
            // Pulled onto a reference line, unless Alt is held.
            let snap = !ui.input(|i| i.modifiers.alt);
            self.snap_the_move(page, view.scale, snap);
            // A turn snaps to whole steps of 15 degrees while Shift is held.
            self.tab_mut().rotate_snap = ui.input(|i| i.modifiers.shift);
        }

        if response.drag_stopped() {
            if let (Some(grab), Some(sel)) = (self.tab_mut().grab.take(), self.tab_mut().selected.clone()) {
                self.finish_grab(sel, grab, view.scale);
            }
            if let Some(grab) = self.tab_mut().group_grab.take() {
                self.finish_group_grab(grab, view.scale);
            }
            if let Some((start, end)) = self.tab_mut().marquee.take() {
                let extend = ui.input(|i| i.modifiers.shift);
                self.select_group_in(page, start, end, extend);
            }
        }

        if response.clicked() {
            if ui.input(|i| i.modifiers.shift) {
                self.extend_selection_at(page, at);
            } else {
                self.tab_mut().group = Vec::new();
                self.select_thing_at(page, at);
            }
        }
    }

    /// A placed signature's own pointer handling, with **no tool armed** —
    /// click to select, drag the body to move, drag a handle to resize.
    /// Mirrors [`Self::interact_objects`] in shape, but a signature is
    /// never page content (see [`SignatureSelected`]), so it cannot reuse
    /// that function's `move_thing`/`scale_thing` underneath, and — unlike
    /// the object tool, which owns the pointer outright once armed — must
    /// say whether it actually did anything: `true` means the caller stops
    /// here, `false` means nothing here was relevant and normal handling
    /// (text selection, in practice) should carry on as if this had never
    /// been called.
    pub(crate) fn interact_signatures(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        page: usize,
        at: AppPoint,
        view: PageView,
    ) -> bool {
        // Read before the hover block below can overwrite it — see
        // `Self::signature_hover_handle`'s own doc for why a drag that has
        // just started must be classified against *that*, not against a
        // fresh hit-test at `at`.
        let remembered_handle = self.tab_mut().signature_hover_handle;

        if self.tab_mut().signature_grab.is_none() {
            if let Some(rect) = self.tab().signature_selected.as_ref().filter(|s| s.page == page).map(|s| s.rect) {
                let handle = self.signature_handle_at(at, view);
                self.tab_mut().signature_hover_handle = handle;
                if let Some(handle) = handle {
                    ui.output_mut(|o| o.cursor_icon = handle.cursor());
                } else if Self::point_in_rect(at, &rect) {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                }
            } else {
                self.tab_mut().signature_hover_handle = None;
            }
        }

        if response.drag_started() {
            // **Not a fresh hit-test at `at`.** `egui` only decides a press
            // has become a drag once the pointer has moved past its own
            // threshold, and by then `at` has already slid off an
            // eight-pixel handle and onto the rect it sits on — silently
            // turning an aimed resize into a body move. `remembered_handle`
            // is whatever was under the pointer the frame before that
            // slide, which for a real press is exactly where it went down.
            let on_handle = self.tab_mut()
                .signature_selected
                .as_ref()
                .filter(|s| s.page == page)
                .and_then(|_| remembered_handle);
            let on_body = self.tab_mut()
                .signature_selected
                .as_ref()
                .is_some_and(|s| s.page == page && Self::point_in_rect(at, &s.rect));
            if on_handle.is_some() || on_body {
                self.tab_mut().signature_grab = Some(Grab { handle: on_handle, from: at, by: (0.0, 0.0) });
                return true;
            }
            // A drag starting fresh on an unselected signature selects it
            // and carries the gesture on — the object tool's own rule for
            // the same reason: one gesture, not two.
            if let Some((index, rect, rotation)) = self.signature_at(page, at) {
                self.tab_mut().signature_selected = Some(SignatureSelected { page, index, rect, rotation });
                self.drop_object_selection();
                self.tab_mut().signature_grab = Some(Grab { handle: None, from: at, by: (0.0, 0.0) });
                return true;
            }
        }

        if response.dragged() {
            if let Some(grab) = self.tab_mut().signature_grab.as_mut() {
                grab.by = ((at.x - grab.from.x) as f32, (at.y - grab.from.y) as f32);
                ui.output_mut(|o| {
                    o.cursor_icon = match grab.handle {
                        Some(h) => h.cursor(),
                        None => egui::CursorIcon::Grabbing,
                    }
                });
                return true;
            }
        }

        if response.drag_stopped() {
            if let (Some(grab), Some(sel)) = (self.tab_mut().signature_grab.take(), self.tab_mut().signature_selected.clone()) {
                self.finish_signature_grab(sel, grab);
                return true;
            }
        }

        if response.clicked() {
            if let Some((index, rect, rotation)) = self.signature_at(page, at) {
                self.tab_mut().signature_selected = Some(SignatureSelected { page, index, rect, rotation });
                self.drop_object_selection();
                self.say_info("signature selected — drag to move, drag a handle to resize, drag the ring above it to turn.");
                return true;
            }
            // Clicked elsewhere: deselect, but do not swallow the click —
            // it may still be a text cursor or a click somewhere else meant
            // for it, and the caller finds out by getting `false` back.
            self.tab_mut().signature_selected = None;
        }

        false
    }

    /// The same as [`Self::interact_signatures`], for a plain placed
    /// picture — see [`Self::placed_image_selected`]. Tried after
    /// signatures and before the object tool, at the same call site, so a
    /// placed picture and a placed signature never fight over one click.
    pub(crate) fn interact_placed_images(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        page: usize,
        at: AppPoint,
        view: PageView,
    ) -> bool {
        let remembered_handle = self.tab_mut().placed_image_hover_handle;

        if self.tab_mut().placed_image_grab.is_none() {
            if let Some(rect) = self.tab().placed_image_selected.as_ref().filter(|s| s.page == page).map(|s| s.rect) {
                let handle = self.placed_image_handle_at(at, view);
                self.tab_mut().placed_image_hover_handle = handle;
                if let Some(handle) = handle {
                    ui.output_mut(|o| o.cursor_icon = handle.cursor());
                } else if Self::point_in_rect(at, &rect) {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                }
            } else {
                self.tab_mut().placed_image_hover_handle = None;
            }
        }

        if response.drag_started() {
            let on_handle = self.tab_mut()
                .placed_image_selected
                .as_ref()
                .filter(|s| s.page == page)
                .and_then(|_| remembered_handle);
            let on_body = self.tab_mut()
                .placed_image_selected
                .as_ref()
                .is_some_and(|s| s.page == page && Self::point_in_rect(at, &s.rect));
            if on_handle.is_some() || on_body {
                self.tab_mut().placed_image_grab = Some(Grab { handle: on_handle, from: at, by: (0.0, 0.0) });
                return true;
            }
            if let Some((index, rect, rotation)) = self.placed_image_at(page, at) {
                self.tab_mut().placed_image_selected = Some(PlacedImageSelected { page, index, rect, rotation });
                self.drop_object_selection();
                self.tab_mut().placed_image_grab = Some(Grab { handle: None, from: at, by: (0.0, 0.0) });
                return true;
            }
        }

        if response.dragged() {
            if let Some(grab) = self.tab_mut().placed_image_grab.as_mut() {
                grab.by = ((at.x - grab.from.x) as f32, (at.y - grab.from.y) as f32);
                ui.output_mut(|o| {
                    o.cursor_icon = match grab.handle {
                        Some(h) => h.cursor(),
                        None => egui::CursorIcon::Grabbing,
                    }
                });
                return true;
            }
        }

        if response.drag_stopped() {
            if let (Some(grab), Some(sel)) = (self.tab_mut().placed_image_grab.take(), self.tab_mut().placed_image_selected.clone()) {
                self.finish_placed_image_grab(sel, grab);
                return true;
            }
        }

        if response.clicked() {
            if let Some((index, rect, rotation)) = self.placed_image_at(page, at) {
                self.tab_mut().placed_image_selected = Some(PlacedImageSelected { page, index, rect, rotation });
                self.drop_object_selection();
                self.say_info("picture selected — drag to move, drag a handle to resize, drag the ring above it to turn.");
                return true;
            }
            self.tab_mut().placed_image_selected = None;
        }

        false
    }

    /// **A box over every paragraph Edit Text can open**, so what a click will
    /// take is something to see rather than to find out: the paragraph under
    /// the pointer drawn stronger, and a run of lines the page draws as shapes
    /// — which no click can open — dashed and grey, the gap in the paragraph
    /// it cut. The boxes are [`block_input::piece_boxes`]: the very pieces a
    /// click opens, so they cannot disagree with it.
    ///
    /// Only with Edit Text in hand, and only on the page the pointer is over:
    /// the page's paragraphs are read once and kept (`page_blocks`, the read a
    /// click uses, so the first click on a boxed page is free), and a page too
    /// heavy to read for paragraphs is not boxed at all.
    pub(crate) fn draw_paragraph_boxes(
        &mut self,
        ui: &mut egui::Ui,
        page: usize,
        rect: egui::Rect,
        view: PageView,
        hover: Option<egui::Pos2>,
    ) {
        let Some(pointer) = hover.filter(|at| rect.contains(*at)) else { return };
        let painter = ui.painter_at(rect);
        for (outline, style) in self.paragraph_boxes(page, view, pointer) {
            match style {
                BoxStyle::Drawn => {
                    let grey = egui::Stroke::new(1.0, ui.visuals().weak_text_color());
                    let path = [
                        outline.left_top(),
                        outline.right_top(),
                        outline.right_bottom(),
                        outline.left_bottom(),
                        outline.left_top(),
                    ];
                    painter.extend(egui::Shape::dashed_line(&path, grey, 4.0, 3.0));
                }
                BoxStyle::Lit => {
                    painter.rect_filled(outline, egui::CornerRadius::same(2), theme::violet().gamma_multiply(0.10));
                    painter.rect_stroke(
                        outline,
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(1.5, theme::violet()),
                        egui::StrokeKind::Outside,
                    );
                }
                BoxStyle::Plain => {
                    painter.rect_stroke(
                        outline,
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(1.0, theme::violet().gamma_multiply(0.45)),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
    }

    /// What [`Self::draw_paragraph_boxes`] draws, on screen, with the pointer at
    /// `pointer`: empty unless Edit Text is in hand and the page is one that is
    /// read for paragraphs.
    ///
    /// Apart from the work of reading the page, this is a function of its
    /// arguments, which is what lets a test say what is boxed and what is lit
    /// without painting anything.
    pub(crate) fn paragraph_boxes(&mut self, page: usize, view: PageView, pointer: egui::Pos2) -> Vec<(egui::Rect, BoxStyle)> {
        let in_hand = self.tab().tool.as_ref().is_some_and(|t| matches!(t.kind, Tool::PickText));
        if !in_hand || self.page_weight(page).filter(crate::page_is_heavy).is_some() {
            return Vec::new();
        }
        let Ok((blocks, _)) = self.page_blocks(page) else { return Vec::new() };

        // While a box is open, the others stay outlined but nothing is lit:
        // the pointer is for the editor.
        let editing = self.tab().editing_run.is_some();
        let under = view.to_page(pointer);
        let boxes = pagify_shell::block_input::piece_boxes(&blocks);
        let inside = |b: &pagify_shell::block_input::PieceBox| {
            let (x, y) = (under.x as f32, under.y as f32);
            x >= b.rect.left && x <= b.rect.right && y >= b.rect.top && y <= b.rect.bottom
        };
        let lit = if editing { None } else { boxes.iter().position(|b| !b.drawn && inside(b)) };

        // A little air, so the box is not struck through by the first and last lines' own ink.
        const AIR_PT: f32 = 2.0;
        boxes
            .iter()
            .enumerate()
            .map(|(i, piece)| {
                let corner = |x: f32, y: f32| view.to_screen(AppPoint::new(x as f64, y as f64));
                let outline = egui::Rect::from_min_max(
                    corner(piece.rect.left - AIR_PT, piece.rect.top - AIR_PT),
                    corner(piece.rect.right + AIR_PT, piece.rect.bottom + AIR_PT),
                );
                let style = if piece.drawn {
                    BoxStyle::Drawn
                } else if Some(i) == lit {
                    BoxStyle::Lit
                } else {
                    BoxStyle::Plain
                };
                (outline, style)
            })
            .collect()
    }

    /// The selection's outline, its handles, and — mid-drag — where it is
    /// going.
    pub(crate) fn draw_object_selection(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        self.draw_move_guides(ui, page, view);
        let Some(sel) = self.tab_mut().selected.clone().filter(|s| s.page == page) else { return };
        let to_screen = |r: &pdf_core::document::Rect| {
            egui::Rect::from_min_max(
                view.to_screen(AppPoint::new(r.left as f64, r.top as f64)),
                view.to_screen(AppPoint::new(r.right as f64, r.bottom as f64)),
            )
        };
        let painter = ui.painter();
        let outline = to_screen(&sel.rect);

        // Where it is now.
        painter.rect_stroke(
            outline,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.5, theme::violet()),
            egui::StrokeKind::Outside,
        );

        // **Being turned: the outline as it will be, and by how much.** Asked for
        // with a screenshot of what a design program shows — the shape turned
        // about its middle, a line from the middle to the pointer, and the angle
        // in a small label beside it.
        if let Some(grab) = self.tab().grab.clone().filter(|g| g.handle == Some(Handle::Rotate)) {
            let degrees = Self::object_turn(&sel.rect, &grab, self.tab().rotate_snap);
            let centre = view.to_screen(AppPoint::new(
                ((sel.rect.left + sel.rect.right) / 2.0) as f64,
                ((sel.rect.top + sel.rect.bottom) / 2.0) as f64,
            ));
            let corners = [
                egui::pos2(outline.left(), outline.top()),
                egui::pos2(outline.right(), outline.top()),
                egui::pos2(outline.right(), outline.bottom()),
                egui::pos2(outline.left(), outline.bottom()),
            ];
            let (s, c) = degrees.to_radians().sin_cos();
            // Turned clockwise on a y-down screen: the plain rotation matrix.
            let turned: Vec<egui::Pos2> = corners
                .iter()
                .map(|p| {
                    let d = *p - centre;
                    centre + egui::vec2(d.x * c - d.y * s, d.x * s + d.y * c)
                })
                .collect();
            let tint = egui::Stroke::new(1.5, theme::violet_bright());
            painter.add(egui::Shape::convex_polygon(turned, theme::violet().gamma_multiply(0.10), tint));
            let pointer = view.to_screen(AppPoint::new(grab.from.x + grab.by.0 as f64, grab.from.y + grab.by.1 as f64));
            painter.line_segment([centre, pointer], egui::Stroke::new(1.0, theme::violet()));
            Self::draw_angle_label(painter, pointer, degrees);
            return;
        }

        // Where it is going, while it is being dragged.
        if let Some(grab) = &self.tab_mut().grab {
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

        // The handles, at a fixed size on screen whatever the zoom.
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

        // The rotate handle above the top edge, joined to it by a stem, and the
        // small diamond in the middle that it turns about.
        let rotate_screen = Self::rotate_handle_screen_pos(&sel.rect, view);
        let top_centre = view.to_screen(AppPoint::new(
            ((sel.rect.left + sel.rect.right) / 2.0) as f64,
            sel.rect.top as f64,
        ));
        painter.line_segment([top_centre, rotate_screen], egui::Stroke::new(1.0, theme::violet()));
        Self::draw_rotate_icon(painter, rotate_screen);
        let middle = outline.center();
        painter.add(egui::Shape::convex_polygon(
            vec![
                middle + egui::vec2(0.0, -4.0),
                middle + egui::vec2(4.0, 0.0),
                middle + egui::vec2(0.0, 4.0),
                middle + egui::vec2(-4.0, 0.0),
            ],
            egui::Color32::WHITE,
            egui::Stroke::new(1.0, theme::violet()),
        ));
    }

    /// `cell_width` is how wide the thumbnail is drawn: the rail passes its own
    /// width, the grid a share of its own — **a width given, not read from
    /// `available_width`**, because inside the grid's wrapped row that is
    /// whatever is left of the row, which made every thumbnail in it fill the
    /// rest of the line.
    pub(crate) fn draw_thumbnail_cell(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        page: usize,
        modifiers: egui::Modifiers,
        cell_rects: &mut Vec<(usize, egui::Rect)>,
        cell_width: f32,
    ) -> Option<usize> {
        let current = page == self.tab_mut().page;
        let selected = self.tab_mut().organize_selected.contains(&page);
        // **Reported from use, with a screenshot, twice.** First "fill the
        // thumbnails in the ribbon, it's too small", which was answered by
        // drawing the same seventy-pixel bitmap larger. Then "the quality of
        // the preview in the thumbnail is terrible… this is unacceptable":
        // two to three times its size, it was a blur. The thumbnail is now
        // *rendered* at the size it is shown — see `thumb_for`.
        let width = cell_width.max(1.0);
        let width_px = Self::thumb_width_px(width, ctx.pixels_per_point());
        // The cell's height comes from the page's own proportions, not from a
        // bitmap, so a page that is nowhere near the screen costs nothing but
        // its place in the list.
        let aspect = self
            .tab()
            .doc
            .as_ref()
            .and_then(|d| d.strip.size_of(page))
            .map(|(w, h)| h / w.max(1.0))
            .unwrap_or(std::f32::consts::SQRT_2);
        let size = egui::vec2(width, width * aspect);
        // Only what is on screen — and a screenful's margin either side, so a
        // scroll does not show blanks — is rendered. It used to render every
        // page of the document on the first frame, which at this size would
        // be a long freeze on a large one.
        let near = egui::Rect::from_min_size(ui.next_widget_position(), size)
            .intersects(ui.clip_rect().expand2(egui::vec2(0.0, 240.0)));
        let texture = if near { self.thumb_for(ctx, page, width_px) } else { None };
        let response = match &texture {
            Some(texture) => ui.add(
                egui::Image::new(texture).fit_to_exact_size(size).sense(egui::Sense::click_and_drag()),
            ),
            None => {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                if near {
                    // A render that failed is a visible blank, not a hole.
                    ui.painter().rect_filled(rect, 2.0, theme::raised());
                }
                response
            }
        };
        if selected || current {
            ui.painter().rect_stroke(
                response.rect.expand(2.0),
                2.0,
                egui::Stroke::new(2.0, theme::violet()),
                egui::StrokeKind::Outside,
            );
            // The mockup's own ring (`ring-2 ring-brand-100`, `code.html:326`)
            // — a second, wider, fainter outline outside the first, so the
            // active/selected page reads as *framed* rather than just
            // outlined. Opacity rather than a second named colour: it has to
            // sit correctly on both the dark and light palettes without its
            // own pair of theme constants for a single decorative line.
            ui.painter().rect_stroke(
                response.rect.expand(5.0),
                3.0,
                egui::Stroke::new(3.0, theme::violet().gamma_multiply(0.35)),
                egui::StrokeKind::Outside,
            );
        }
        cell_rects.push((page, response.rect));

        let mut jump = None;
        if response.clicked() {
            let command_id = self.command_id();
            ui.memory_mut(|m| m.surrender_focus(command_id));
            let plain = !modifiers.command && !modifiers.shift;
            let tab = self.tab_mut();
            Self::apply_organize_click(&mut tab.organize_selected, &mut tab.organize_anchor, page, modifiers);
            if plain {
                jump = Some(page);
            }
        }
        // Dragging an unselected thumbnail selects just that one first —
        // the same thing Explorer does when you drag an item you had not
        // already clicked.
        if response.drag_started() {
            let command_id = self.command_id();
            ui.memory_mut(|m| m.surrender_focus(command_id));
            if !self.tab_mut().organize_selected.contains(&page) {
                self.tab_mut().organize_selected = vec![page];
                self.tab_mut().organize_anchor = Some(page);
            }
            let moving = self.tab_mut().organize_selected.clone();
            self.tab_mut().organize_drag = Some(OrganizeDrag {
                moving,
                pointer_started_at: response.interact_pointer_pos().unwrap_or(response.rect.center()),
            });
        }
        let size_label = self
            .tab_mut()
            .doc
            .as_ref()
            .and_then(|d| d.strip.size_of(page))
            .map(|(w, h)| Self::paper_size_label(w, h));
        // Truncated, not allowed to run on: a long size name beside a three-
        // digit page number is wider than a narrow cell, and a cell wider than
        // the width it was given is what the Organize grid must never have —
        // see `draw_organize_grid`.
        ui.horizontal(|ui| {
            ui.small(format!("{}", page + 1));
            if let Some(label) = size_label {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(label).size(9.0).weak()).truncate());
            }
        });
        jump
    }

    /// The Organize grid: a wrapped, multi-column thumbnail view occupying
    /// the same panel slot the plain scrolling rail does, while
    /// [`Self::organize_open`] is set — see [`Self::toggle_organize_grid`].
    /// A bigger surface for working across many pages at once; the plain
    /// "Pages" rail supports the same select/drag/copy/paste/delete
    /// operations via [`Self::draw_thumbnail_cell`], so opening this is
    /// never required to reorganize a document, only convenient for it.
    pub(crate) fn draw_organize_grid(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let modifiers = ctx.input(|i| i.modifiers);
        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let released = ctx.input(|i| i.pointer.any_released());
        let mut cell_rects: Vec<(usize, egui::Rect)> = Vec::new();
        let mut jump_to = None;

        egui::Panel::left(ORGANIZE_GRID_PANEL)
            .resizable(true)
            .default_size(220.0)
            .size_range(104.0..=600.0)
            .frame(egui::Frame::new().fill(theme::paper()).inner_margin(egui::Margin::symmetric(8, 8)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Organize");
                    if ui.small_button("Close").clicked() {
                        self.toggle_organize_grid();
                    }
                });
                ui.add_space(4.0);
                // **Reported from use: "when I click Thumbnail View the
                // thumbnail ribbon glitches out and starts zooming in and
                // out", and "it only displays five pages".** Two faults, one
                // cause: this was a wrapped layout of cells that nothing
                // made fit.
                //
                // A panel is as wide as what is in it, and a vertical scroll
                // area follows its content's width, so the grid's width
                // became its own input. Its cells were worked out to fill the
                // width exactly — but each was followed by a gap and a spacer
                // that the arithmetic never counted, so the row was always 14
                // points wider than the width it was made for. The panel grew
                // by 14 points a frame, the cells grew with it, and when the
                // column count at last changed the cells dropped to a third
                // of their size and it began again: a page going 135, 149,
                // 163, 177, 191, 95 points wide, six frames to the cycle,
                // without end (one page open, as in the report).
                //
                // And a wrapped layout does not wrap a cell that is a layout
                // of its own, nor a gap that is only a cursor move: every page
                // went along one row, off the right edge of the panel, where
                // only the first few could be seen.
                //
                // So: the width is read once, here, outside the scroll area;
                // the cells are worked out so a row can never exceed it
                // ([`Self::grid_cell_width`]); the rows are built row by row
                // instead of left to wrap; and the scroll area scrolls
                // sideways too, so that should something ever be wider than
                // it was given it scrolls and does not push the panel out.
                let width = ui.available_width();
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    let count = self.tab_mut().doc.as_ref().map(|d| d.page_count).unwrap_or(0);
                    let columns = Self::grid_columns(width);
                    let cell_width = Self::grid_cell_width(width, columns);
                    ui.spacing_mut().item_spacing = egui::vec2(GRID_GAP_PT, GRID_GAP_PT);
                    for first in (0..count).step_by(columns) {
                        ui.horizontal_top(|ui| {
                            for page in first..(first + columns).min(count) {
                                ui.vertical(|ui| {
                                    ui.set_max_width(cell_width);
                                    if let Some(page) = self.draw_thumbnail_cell(
                                        ctx,
                                        ui,
                                        page,
                                        modifiers,
                                        &mut cell_rects,
                                        cell_width,
                                    ) {
                                        jump_to = Some(page);
                                    }
                                });
                            }
                        });
                    }
                    Self::draw_drop_indicator(
                        ui,
                        self.tab_mut().organize_drag.is_some(),
                        &cell_rects,
                        pointer_pos,
                    );
                });
            });

        if let Some(page) = jump_to {
            self.act(Verb::Page(PageTarget::Number(page + 1)));
        }
        self.finish_thumbnail_drag(&cell_rects, pointer_pos, released);
    }

    pub(crate) fn draw_pages(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, command_id: egui::Id) {
        let mut zoom = self.resolved_zoom();
        let (strip_height, strip_width, page_count) = {
            let doc = self.tab_mut().doc.as_ref().expect("checked");
            (doc.strip.height_pt(), doc.strip.width_pt(), doc.page_count)
        };

        // Zoom at the pointer, before the scroll area gets the wheel.
        //
        // Anchored rather than centred: zooming towards the middle of the
        // window moves whatever you were looking at off it, and on a page of
        // small type that is the whole reason you were zooming. The point under
        // the cursor is the one the user is asking about, so it is the one that
        // stays still.
        //
        // **The window the pages are about to be drawn in, not the one they were
        // drawn in last frame.** It was read back from the previous frame, so
        // for one frame after the page area changed — a panel docking, the rail
        // opening, a window being dragged — the page was centred for the old
        // width and snapped a beat later. The scroll area is the first thing
        // added to this `ui` and floating scroll bars take no room, so what is
        // left of it is exactly what the scroll area gets.
        let viewport = ui.available_rect_before_wrap();
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| viewport.contains(*p));
        zoom = self.zoom_at_pointer(ui, pointer, zoom);

        self.note_zoom_and_collect_renders(ctx, zoom);

        // Every use of `zoom` above this line needed the *logical* value —
        // the pinch handling just above reads and writes `ZoomMode::Factor`
        // directly, so correcting `zoom` before that would have baked
        // `DISPLAY_DPI_SCALE` into the stored factor and compounded it on
        // every further pinch. Everything from here down draws or hit-tests
        // the page, which wants the on-screen value instead — see
        // `DISPLAY_DPI_SCALE`'s own doc for why they differ at all.
        zoom *= Self::DISPLAY_DPI_SCALE;

        // One scroll state per document. With a shared one, switching tabs kept
        // the other document's pixel offset under this one's pages.
        let scroll_id = self.tab().doc.as_ref().map_or(0, |d| d.id);
        let mut area =
            egui::ScrollArea::both().id_salt(("pages", scroll_id)).auto_shrink([false, false]);
        // Whether this frame dictated the offset rather than observing it.
        let (area, forced) =
            self.take_forced_scroll(area, viewport, zoom, strip_width, strip_height);
        let scroll = area
            .show(ui, |ui| {
                let content =
                    egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
                let (_, canvas) = ui.allocate_space(content);

                // Centred while it fits, pinned to the corner once it does not.
                //
                // A scroll area lays its content out from the top-left, so a
                // page smaller than the window sat against the left edge with
                // all the empty space on one side. Half the slack on each side
                // is what a reader expects of a page on a desk.
                //
                // This does not disturb zoom anchoring: the padding is only
                // non-zero on an axis where the content fits, and an axis that
                // fits cannot scroll, so there was never an offset to hold on
                // it anyway.
                let pad = ((viewport.size() - content) * 0.5).max(egui::Vec2::ZERO);

                // Which slice of the strip the window is over, in page points.
                let visible_top =
                    ((ui.clip_rect().top() - canvas.top() - pad.y) / zoom).max(0.0);
                let visible_bottom = (ui.clip_rect().bottom() - canvas.top() - pad.y) / zoom;
                let visible = {
                    let doc = self.tab_mut().doc.as_ref().expect("checked");
                    doc.strip.visible(visible_top, visible_bottom)
                };

                // The page you are looking at is the page you are working on.
                //
                // Nothing kept `self.tab_mut().page` in step with the scroll: it moved
                // only for `page next` and friends. On a one-page fixture that
                // is invisibly correct, and on a 149-page catalogue it means
                // the pointer talks to page 1 while the reader is on page 40 —
                // so clicking and dragging do nothing at all, which is exactly
                // what "selection does not work" looked like.
                //
                // Not while the pointer is down: re-deciding the current page
                // in the middle of a drag would drop the selection being made.
                if self.tab_mut().settling > 0 {
                    self.tab_mut().settling -= 1;
                } else if !ui.ctx().input(|i| i.pointer.any_down()) {
                    let clip = ui.clip_rect();
                    let mut best: Option<(usize, f32)> = None;
                    for page in visible.clone() {
                        let doc = self.tab_mut().doc.as_ref().expect("checked");
                        let Some(top) = doc.strip.top_of(page) else { continue };
                        let Some((_, h)) = doc.strip.frame_of(page) else { continue };
                        let y0 = canvas.top() + pad.y + 12.0 + top * zoom;
                        let y1 = y0 + h * zoom;
                        // How much of the window this page fills.
                        let shown = (y1.min(clip.bottom()) - y0.max(clip.top())).max(0.0);
                        if best.map_or(true, |(_, b)| shown > b) {
                            best = Some((page, shown));
                        }
                    }
                    if let Some((page, _)) = best {
                        // The selection is *not* dropped here. It belongs to
                        // the page it was made on, which `selection_page`
                        // remembers, and scrolling past is not a decision to
                        // discard it.
                        self.tab_mut().page = page;
                    }
                }

                let hover = ui.ctx().input(|i| i.pointer.hover_pos());
                for page in visible.clone() {
                    let (top, (w, h)) = {
                        let doc = self.tab_mut().doc.as_ref().expect("checked");
                        (doc.strip.top_of(page).unwrap_or(0.0), doc.strip.frame_of(page).unwrap_or((612.0, 792.0)))
                    };
                    // Placement comes from the strip, which is what knows
                    // whether this page shares its row with another.
                    let left = {
                        let doc = self.tab_mut().doc.as_ref().expect("checked");
                        doc.strip.left_of(page).unwrap_or((strip_width - w) / 2.0)
                    };
                    let origin = egui::pos2(
                        canvas.left() + pad.x + 12.0 + left * zoom,
                        canvas.top() + pad.y + 12.0 + top * zoom,
                    );

                    // Past the GPU's limit the page is still *drawn* at the
                    // asked-for size and simply gets softer, which is the right
                    // trade. `texture_for` applies that cap.
                    let device_scale = raster_scale(zoom * ctx.pixels_per_point());
                    if let Some(texture) = self.texture_for(ctx, page, device_scale) {
                        // **The page's true size at this zoom, not the
                        // texture's.** The raster is quantised so that a pinch
                        // does not re-render sixty times a second; drawing it at
                        // its own size passes that quantisation straight through
                        // to the screen, so the page jumps between a handful of
                        // sizes instead of zooming. Letting the GPU scale the
                        // texture to the asked-for rectangle is the entire point
                        // of quantising it.
                        //
                        // It was also wrong: `PageView` below maps clicks at the
                        // true `zoom`, so wherever the raster scale differed from
                        // it, the picture and the hit-testing disagreed.
                        let size = egui::vec2(w * zoom, h * zoom);
                        let rect = egui::Rect::from_min_size(origin, size);
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );

                        let view = PageView { origin, scale: zoom };
                        self.draw_detail_overlay(ctx, ui, page, view, rect, device_scale);
                        self.draw_text_highlights(ui.painter(), page, view);
                        if let Some(layer) = self.tab_mut().markup.existing(page) {
                            overlay::draw_layer(
                                ui.painter(),
                                layer,
                                view,
                                theme::markup(),
                                theme::selected(),
                            );
                        }
                        // **A bookmarked page said so nowhere on the page
                        // itself.** Reported from use: adding one gave no
                        // lasting sign that it had worked short of reopening
                        // the panel. A fixed screen size rather than one
                        // that scales with zoom — the same reasoning a
                        // window's own corner badges are always the same
                        // size regardless of how far the content under them
                        // is zoomed.
                        if self.tab_mut().bookmarked_pages.contains(&page) {
                            let size = 22.0_f32;
                            let inset = 6.0_f32;
                            let badge = egui::Rect::from_min_size(
                                egui::pos2(rect.right() - inset - size, rect.top() + inset),
                                egui::vec2(size, size),
                            );
                            ui.painter().rect_filled(badge, egui::CornerRadius::ZERO, theme::violet());
                            ui.painter().text(
                                badge.center(),
                                egui::Align2::CENTER_CENTER,
                                "\u{E8E7}",
                                icon_font(size * 0.62),
                                egui::Color32::WHITE,
                            );
                        }
                        if page == self.tab_mut().page {
                            self.tab_mut().last_view = Some(view);
                        }
                        // Zoom anchors against the page under the cursor, which
                        // on a scrolling strip is often not the current one.
                        // Anchoring with another page's mapping puts the fixed
                        // point on the wrong page and the view slides.
                        if hover.is_some_and(|p| rect.contains(p)) {
                            self.tab_mut().hover_view = Some((page, view));
                        }
                        // Every visible page is live, not just the current one:
                        // you interact with what you are pointing at, and only
                        // the page under the pointer will answer anyway.
                        //
                        // The exception is a tool that is part-way through. Its
                        // picks belong to the page it was armed on, and letting
                        // a later click on a different page join them would put
                        // one end of the line on page 40 and the other on page
                        // 41 — in coordinates that mean nothing on either.
                        // **A tool that has collected nothing has not started.**
                        //
                        // The rule below binds a part-way pick to its page, for
                        // a good reason. But it also silently swallowed the
                        // *first* click of a one-click tool armed while another
                        // page was current — on a scrolling strip with two pages
                        // in view, arming and then clicking the other one did
                        // nothing whatever: no mark, no message, nothing to
                        // read. So a pick holding no points and no objects yet
                        // starts on whichever page is actually clicked.
                        if let Some(armed) = &mut self.tab_mut().tool {
                            if armed.page != page
                                && armed.points.is_empty()
                                && armed.objects.is_empty()
                                && hover.is_some_and(|at| rect.contains(at))
                            {
                                armed.page = page;
                            }
                        }
                        let owns = self.tab_mut().tool.as_ref().map_or(true, |p| p.page == page);
                        if owns {
                            self.interact(ui, rect, view, page, command_id);
                        }
                        // **After the page's own interaction, not before it.**
                        // egui gives a click to the last widget registered over
                        // that spot, and the page covers every badge on it — so
                        // declaring the badges first meant the page swallowed
                        // every click and a padlock could never be pressed.
                        self.draw_lock_badges(ui, page, view);
                        self.draw_picked_layer(ui, page, view);
                        self.draw_object_selection(ui, page, view);
                        self.draw_group_selection(ui, page, view);
                        self.draw_markup_selection(ui, page, view);
                        self.draw_signature_selection(ui, page, view);
                        self.draw_placed_image_selection(ui, page, view);
                        self.draw_paste_ghost(ui, page, rect, view);
                        // On top of the page and its badges, under the editor:
                        // a tool part-way through is the most recent thing the
                        // reader did and the thing they are aiming with.
                        self.draw_pending_preview(ui, page, view, hover);
                        // Under the editor, over the page: what a click on
                        // Edit Text would open, boxed.
                        self.draw_paragraph_boxes(ui, page, rect, view, hover);
                        // The editor, for the same reason, learned twice.
                        //
                        // Reported from use: "if i click somewhere else the
                        // cursor is gone and i cant bring it back… then i cant
                        // apply the change." Declared before the page, its
                        // field and its Apply button were under a widget that
                        // takes every click over them — so the caret could be
                        // given once, by asking for it, and never again by
                        // clicking. Drawn last, it is simply clickable.
                        self.draw_run_editor(ui, page, view);
                        self.draw_new_text_box(ui, page, view);
                    }
                }

                visible
            });

        let _ = forced;
        self.settle_strip_after_scroll(
            ctx, zoom, page_count, scroll.state.offset, scroll.inner_rect, &scroll.inner,
        );
    }

    /// Zoom at the pointer: the point under the cursor stays under it, and
    /// the scroll offset is anchored to it rather than to the middle of the
    /// window — see the comments this moved away from in `draw_pages`.
    /// Returns the (possibly corrected) zoom for the rest of the frame.
    fn zoom_at_pointer(
        &mut self,
        ui: &mut egui::Ui,
        pointer: Option<egui::Pos2>,
        mut zoom: f32,
    ) -> f32 {
        if let Some(p) = pointer {
            // egui folds ⌘/Ctrl-scroll and a trackpad pinch into the same
            // number, which is right: they are one gesture with two spellings.
            let factor = ui.input(|i| i.zoom_delta());
            if (factor - 1.0).abs() > 0.001 {
                let before = zoom;
                let after = (before * factor).clamp(0.05, Self::MAX_ZOOM);
                if (after - before).abs() > f32::EPSILON {
                    // Anchored against where the page **was actually drawn**,
                    // not against a model of where it ought to be.
                    //
                    // Reconstructing the content position from the viewport,
                    // the scroll offset and the strip padding means encoding
                    // the layout twice, and any term missed from the copy shows
                    // up as the page creeping away from the cursor — which it
                    // did, vertically, by a different amount at every scale.
                    // `last_view` is the mapping the previous frame really
                    // used, so there is nothing left to get wrong.
                    // The page under the pointer, or the current one when the
                    // pointer is beside the page rather than on it — the strip
                    // is wider than the paper.
                    let anchor_on =
                        self.tab_mut().hover_view.or_else(|| self.tab_mut().last_view.map(|v| (self.tab_mut().page, v)));
                    if let Some((index, view)) = anchor_on {
                        // Where this page sits in the strip, in strip points.
                        //
                        // This is the term the anchor was missing. A page's
                        // origin on screen is
                        //
                        //     viewport - offset + padding + strip_position * zoom
                        //
                        // so changing the zoom moves it **even at a fixed
                        // offset**, by `strip_position * (after - before)`.
                        // Correcting only for the offset leaves exactly that
                        // much error, and it grows with distance down the
                        // document: on the first page `strip_position` is zero
                        // and the anchor looks perfect, which is why every
                        // single-page test passed while a catalogue slid 200pt.
                        let (sx, sy) = self.tab_mut()
                            .doc
                            .as_ref()
                            .map(|d| {
                                let (w, _) = d.strip.frame_of(index).unwrap_or((0.0, 0.0));
                                (
                                    d.strip
                                        .left_of(index)
                                        .unwrap_or((d.strip.width_pt() - w) / 2.0),
                                    d.strip.top_of(index).unwrap_or(0.0),
                                )
                            })
                            .unwrap_or((0.0, 0.0));

                        // `before`/`after` are the *logical* factor — right
                        // for `ZoomMode::Factor` just below, wrong here:
                        // `origin_after`/`with_strip` place things on screen,
                        // which is `view`'s own job, and `view.scale` is
                        // already the on-screen value `DISPLAY_DPI_SCALE`
                        // produces. Found by this exact anchor test failing
                        // once that correction landed — mixing a logical
                        // factor into on-screen arithmetic held the wrong
                        // point still, by exactly the 96/72 the two disagree
                        // by.
                        let (before_screen, after_screen) =
                            (before * Self::DISPLAY_DPI_SCALE, after * Self::DISPLAY_DPI_SCALE);
                        let on_page = view.to_page(p);
                        let origin_after = egui::vec2(
                            p.x - on_page.x as f32 * after_screen,
                            p.y - on_page.y as f32 * after_screen,
                        );
                        let moved = view.origin.to_vec2() - origin_after;
                        let with_strip = egui::vec2(sx, sy) * (after_screen - before_screen);
                        self.tab_mut().anchor_offset = Some(self.tab_mut().scroll_offset + moved + with_strip);
                    }
                    self.tab_mut().zoom = ZoomMode::Factor(after);
                    zoom = after;

                    // The gesture belongs to the zoom. Left in place, the same
                    // wheel also scrolls the strip, and the two fight for the
                    // offset every frame — which reads as the page shuddering
                    // rather than zooming.
                    ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                }
            }
        }
        zoom
    }


    /// The scroll offset this frame has been told to take — a pan, a reveal,
    /// a scroll-to, a zoom anchor or a restored view — applied to the scroll
    /// area. Returns the area and the offset it dictated, if any; the
    /// comments this moved away from in `draw_pages` explain each branch.
    fn take_forced_scroll(
        &mut self,
        mut area: egui::ScrollArea,
        viewport: egui::Rect,
        zoom: f32,
        strip_width: f32,
        strip_height: f32,
    ) -> (egui::ScrollArea, Option<egui::Vec2>) {
        let mut forced: Option<egui::Vec2> = None;
        if let Some(by) = self.tab_mut().pan_by.take() {
            // Clamped to what can actually be scrolled to.
            //
            // Without the upper bound the offset keeps growing past the end of
            // the document while the drag continues; the scroll area clamps
            // what it draws, and the accumulated excess springs back the moment
            // the drag reverses. That is the bounce at the edges.
            let content = egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
            let room = (content - viewport.size()).max(egui::Vec2::ZERO);
            let to = (self.tab_mut().scroll_offset + by).clamp(egui::Vec2::ZERO, room);
            forced = Some(to);
            area = area.scroll_offset(to);
        } else if let Some(Reveal { page, rect }) = self.tab_mut().reveal.take() {
            // A word to show, not a page: scrolled just far enough, on both
            // axes, and never by a change of zoom. `go_to` asked for the page's
            // top along with it — that is what a word in the first screenful
            // settles for, and the request is taken here, or it would fire a
            // frame late and undo this.
            let page_top = self.tab_mut().scroll_to_pt.take();
            self.tab_mut().anchor_offset = None;
            let content = egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
            let room = (content - viewport.size()).max(egui::Vec2::ZERO);
            let at = self.tab().scroll_offset;
            let place = self.tab().doc.as_ref().and_then(|d| Some((d.strip.left_of(page)?, d.strip.top_of(page)?)));
            let to = match place {
                Some((left, top)) => {
                    // Content pixels, from the content's own origin: the
                    // strip's padding, then the page, then the word on it.
                    let pad = STRIP_PAD_PX;
                    let (x0, x1) = (pad + (left + rect.left) * zoom, pad + (left + rect.right) * zoom);
                    let (y0, y1) = (pad + (top + rect.top) * zoom, pad + (top + rect.bottom) * zoom);
                    // `at` was measured at last frame's zoom, which Fit and
                    // Width change with the page. No matter: the word's own
                    // extent is at *this* zoom, so a window that holds it
                    // holds it, and the answer is clamped to the new end.
                    egui::vec2(
                        reveal_axis(at.x, viewport.width(), room.x, x0, x1, None),
                        reveal_axis(at.y, viewport.height(), room.y, y0, y1, page_top.map(|y| y * zoom)),
                    )
                }
                None => egui::vec2(at.x, page_top.map_or(at.y, |y| y * zoom)),
            };
            forced = Some(to);
            area = area.scroll_offset(to);
        } else if let Some(y) = self.tab_mut().scroll_to_pt.take() {
            // 12.0 is the strip's top padding, the same constant the page
            // origins are laid out from.
            area = area.scroll_offset(egui::vec2(self.tab_mut().scroll_offset.x, y * zoom));
        } else if let Some(offset) = self.tab_mut().anchor_offset.take() {
            let content = egui::vec2(strip_width * zoom + 24.0, strip_height * zoom + 24.0);
            let room = (content - viewport.size()).max(egui::Vec2::ZERO);
            let to = offset.clamp(egui::Vec2::ZERO, room);
            forced = Some(to);
            area = area.scroll_offset(to);
        } else if let Some(to) = {
            // Nothing asked to go anywhere, but the zoom, the window or the
            // pages are not what they were last frame: keep the reader at the
            // same place on the page rather than at the same pixel offset.
            let tab = self.tab();
            tab.view.zip(tab.doc.as_ref()).and_then(|(seen, doc)| {
                seen.restored(&doc.strip, zoom, (viewport.width(), viewport.height()))
            })
        } {
            let to = egui::vec2(to.0, to.1);
            forced = Some(to);
            area = area.scroll_offset(to);
        }
        (area, forced)
    }


    /// Note when the zoom last changed — what a new render waits on, see
    /// [`ZOOM_SETTLE_SECS`] — and collect whatever renders have come back.
    /// The comment above the call site in `draw_pages` explains why it is
    /// watched on the zoom itself rather than on the events that move it.
    fn note_zoom_and_collect_renders(&mut self, ctx: &egui::Context, zoom: f32) {
        let now = ctx.input(|i| i.time);
        let tab = self.tab_mut();
        if (tab.last_drawn_zoom - zoom).abs() > 1e-4 {
            tab.last_drawn_zoom = zoom;
            tab.zoom_changed_at = now;
        }
        self.collect_renders(ctx);
    }

    /// Prefetch the neighbouring pages and evict rasters for the ones the
    /// reader has scrolled away from, then record where the strip ended up.
    ///
    /// Observed, not the value asked for — the next zoom anchors from it, and
    /// mixing an intended offset with an observed origin makes the page slide
    /// away as you zoom.
    fn settle_strip_after_scroll(
        &mut self,
        ctx: &egui::Context,
        zoom: f32,
        page_count: usize,
        offset: egui::Vec2,
        inner_rect: egui::Rect,
        visible: &std::ops::Range<usize>,
    ) {
        // Prefetch the neighbours, so a scroll onto them is a copy rather than a
        // render. One per frame: the point is to be ready, not to stall now.
        // Where the strip ended up, so the next zoom can anchor from it.
        //
        // Observed, not the value asked for — and it has to be observed,
        // because the anchor pairs it with `hover_view.origin`, which is where
        // the page was *actually drawn* this frame. Mixing an intended offset
        // with an observed origin means the two describe different moments, and
        // the difference accumulates into the page sliding away as you zoom.
        self.tab_mut().scroll_offset = offset;
        self.tab_mut().viewport_rect = Some(inner_rect);
        // Where the reader is looking now, for the next frame to compare with.
        let seen = self.tab().doc.as_ref().and_then(|doc| {
            pagify_shell::reader::ViewSnapshot::capture(
                &doc.strip,
                zoom,
                (inner_rect.width(), inner_rect.height()),
                (offset.x, offset.y),
            )
        });
        self.tab_mut().view = seen;
                if let Some(target) = prefetch_targets(visible, page_count, 2).first().copied() {
            // The same quantised scale the draw uses. Prefetching at the raw
            // zoom would warm a texture the next frame does not ask for.
            let device_scale = raster_scale(zoom * ctx.pixels_per_point());
            self.warm_texture(ctx, target, device_scale);
        }

        // Evict rasters for pages nowhere near the window.
        if let Some(doc) = &mut self.tab_mut().doc {
            let keep_from = visible.start.saturating_sub(3);
            let keep_to = visible.end + 3;
            doc.caches.evict_outside(keep_from, keep_to);
        }
    }

    /// The selection, and every search hit, drawn over the page.
    ///
    /// Under the marks rather than over them: a highlight is a wash on the
    /// paper, and drawing it on top would dim the ink it is meant to point at.
    /// The shape a half-finished tool would make, following the pointer.
    ///
    /// **Because a tool that shows nothing until it is finished asks you to
    /// aim blind.** Every one of these collects two points or more, and until
    /// the last click there was nothing on screen to say where the first one
    /// had landed or what was being made — reported from use as wanting the
    /// drawing to start at the first click.
    ///
    /// Drawn from the point that has been placed to wherever a click would land
    /// *now*, snapping included, so the preview is the thing that would happen
    /// rather than an impression of it.
    pub(crate) fn draw_pending_preview(
        &mut self,
        ui: &mut egui::Ui,
        page: usize,
        view: PageView,
        hover: Option<egui::Pos2>,
    ) {
        // click either. `PlaceText` previews a rubber-band box once its
        // first corner is down.
        //
        // The actual shape, per `Tool` kind, is `Tool::preview` now
        // (`tool.rs`) — `Pagify-Phase2-BigTasks.md` §2.3 step 4.
        let tab = self.tab();
        let Some(armed) = tab.tool.as_ref() else { return };
        if armed.page != page {
            return;
        }
        let Some(cursor) = hover else { return };
        let at = tab
            .last_snap
            .as_ref()
            .map(|snapped| snapped.at)
            .unwrap_or_else(|| view.to_page(cursor));
        let kind = armed.kind.clone();
        let points = armed.points.clone();
        kind.preview(self, ui, view, &points, at);
    }

    /// A padlock over every sealed object on this page, and the click that
    /// brings one back.
    ///
    /// **The badge is the only thing that says a lock is there.** What was
    /// removed leaves a gap, and a gap on a page reads as a design choice
    /// rather than as something hidden — so the mark has to be visible, has to
    /// sit where the thing was, and has to be the way back.
    pub(crate) fn draw_lock_badges(&mut self, ui: &mut egui::Ui, page: usize, view: PageView) {
        let items = self.locked_items_on(page);
        if items.is_empty() {
            return;
        }

        // Where this page's words are, on screen.
        //
        // **The chequerboard must not cover them.** A full-page photo with a
        // caption over it is an ordinary layout, and filling the image's whole
        // rectangle hid text that was never locked — it looked, exactly as it
        // was reported, like locking the picture had taken the words with it.
        // The words are still on the page; only the picture went.
        let words: Vec<egui::Rect> = self
            .characters(page)
            .map(|chars| {
                chars
                    .line_rects(0..chars.len())
                    .into_iter()
                    .map(|r| {
                        egui::Rect::from_min_max(
                            view.to_screen(AppPoint::new(r.left as f64, r.top as f64)),
                            view.to_screen(AppPoint::new(r.right as f64, r.bottom as f64)),
                        )
                        // A little room, so a square does not clip an ascender.
                        .expand(2.0)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let painter = ui.painter();
        let mut asked: Option<String> = None;

        for item in &items {
            let min = view.to_screen(AppPoint::new(item.rect.left as f64, item.rect.top as f64));
            let max = view.to_screen(AppPoint::new(item.rect.right as f64, item.rect.bottom as f64));
            let area = egui::Rect::from_min_max(min, max);

            // A chequerboard, which every editor uses for "nothing here" — the
            // gap reads as deliberate emptiness rather than as a design
            // choice, and it is unmistakably not part of the document.
            //
            // Clipped to the area and drawn from its own top-left rather than
            // the screen's, so the squares hold still while the page scrolls
            // instead of the pattern swimming under a stationary image.
            // A locked *area* was redacted, so the page already carries the
            // black mark that says so. Drawing anything over it would be a
            // second answer to the same question — and would hide the very
            // thing it was trying to explain. Only the padlock goes on top.
            //
            // **And nothing is painted over a picture that is still there.**
            // The chequerboard says "the picture is gone"; over a picture that
            // was never taken off it said something untrue, and read as a grey
            // panel nobody could select or send back — reported from use, with
            // a screenshot. A stale badge gets a dashed outline and its own
            // words instead, until the lock is finished.
            if item.stale {
                painter.rect_stroke(
                    area,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(2.0, theme::danger()),
                    egui::StrokeKind::Inside,
                );
                let label = egui::Rect::from_min_size(
                    area.left_top() + egui::vec2(4.0, 4.0),
                    egui::vec2(area.width() - 8.0, 16.0),
                );
                painter.rect_filled(label, egui::CornerRadius::ZERO, theme::danger());
                painter.text(
                    label.center(),
                    egui::Align2::CENTER_CENTER,
                    "lock not applied — reopen the file, or `repairlocks`",
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            } else if !item.is_area {
                let squares = painter.with_clip_rect(area.intersect(painter.clip_rect()));

                // **The grid is laid out in page points, not screen pixels.**
                //
                // Reported from use: zooming made bits of the pattern appear
                // and disappear beside the caption. A screen-space grid moves
                // relative to the page as the zoom changes, so which squares
                // fall on a word keeps changing — the flicker was the
                // text-avoidance being recomputed against a moving grid. In
                // page space the answer is the same at every zoom.
                const SQUARE_PT: f32 = 6.0;
                let step = (SQUARE_PT * view.scale as f32).max(2.0);
                let (across, down) = (
                    (area.width() / step).ceil() as i32,
                    (area.height() / step).ceil() as i32,
                );

                // The words to keep clear, in page points, as one rectangle per
                // line — also zoom-independent, and expanded a little so a
                // square never clips an ascender.
                let clear: Vec<egui::Rect> = words
                    .iter()
                    .copied()
                    .filter(|w| w.intersects(area))
                    .collect();

                for row in 0..down {
                    for column in 0..across {
                        let at = area.min + egui::vec2(column as f32 * step, row as f32 * step);
                        let square =
                            egui::Rect::from_min_size(at, egui::Vec2::splat(step)).intersect(area);
                        // The text keeps its own background, whatever the page
                        // draws behind it — the chequerboard says "the picture
                        // is gone", and painting it over words that are still
                        // there would say something untrue.
                        if clear.iter().any(|w| w.intersects(square)) {
                            continue;
                        }
                        squares.rect_filled(
                            square,
                            egui::CornerRadius::ZERO,
                            if (row + column) % 2 == 0 {
                                theme::chequer_light()
                            } else {
                                theme::chequer_dark()
                            },
                        );
                    }
                }
                painter.rect_stroke(
                    area,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, theme::violet()),
                    egui::StrokeKind::Inside,
                );
            }

            // A padlock big enough to hit, but never larger than what it marks
            // — a badge overflowing a small image would cover its neighbours.
            let size = 26.0_f32.min(area.width() * 0.8).min(area.height() * 0.8).max(12.0);
            let badge = egui::Rect::from_center_size(area.center(), egui::vec2(size, size));
            painter.rect_filled(badge, egui::CornerRadius::ZERO, theme::violet());
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "\u{E899}",
                icon_font(size * 0.62),
                egui::Color32::WHITE,
            );

            let hit = ui.interact(
                badge,
                egui::Id::new(("lock-badge", page, item.id.as_str())),
                egui::Sense::click(),
            );
            if hit.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if hit.clicked() {
                asked = Some(item.id.clone());
            }
        }

        if let Some(id) = asked {
            // Always asks, even with a passcode held — see
            // `a_held_passcode_does_not_unlock_anything`.
            self.tab_mut().awaiting_password = Some(Awaiting::UnlockItem(id));
            self.say_info("type the passcode this was locked with, or Escape to give up.");
        }
    }

    pub(crate) fn interact(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        view: PageView,
        page: usize,
        command_id: egui::Id,
    ) {
        let response = ui.interact(rect, egui::Id::new(("page", page)), egui::Sense::click_and_drag());

        // **Click elsewhere applies the paragraph being edited, rather than
        // doing nothing to it.** `draw_run_editor` is drawn after this
        // page's own response is created, so a click that actually lands on
        // the open editor's own `TextEdit` is consumed there and never
        // reaches here at all — by the time this function sees a fresh
        // click or drag on its own response, the pointer genuinely did not
        // land on the editor, on this page or any other. Falling through
        // to the rest of this function's own dispatch afterwards matters:
        // if the same click also lands on a different run while a pick is
        // freshly armed, the old edit commits and the new one opens in the
        // same gesture. Escape still discards, unchanged — only this
        // "clicked away" gesture's meaning changes, from silently doing
        // nothing to committing what was typed.
        //
        // **Whether this click was already answered here** — it must not be
        // answered again by the armed-tool block further down this same call.
        // Reported from use, in the logs: one click away from an editor
        // logged two identical "no text there" errors a few milliseconds
        // apart, because a failed pick re-arms the tool and the armed-tool
        // block then took the same, still-fresh click a second time.
        let mut answered_above = false;
        if (response.clicked() || response.drag_started()) && self.tab_mut().editing_run.is_some() {
            let errors_before = self.errors_said;
            // Applies it — or, an edit the engine already refused and that has not
            // changed since, lets it go. An edit that is refused **stays open** with
            // its words, and then this click is no pick (below) and does nothing else.
            if self.leave_editor_by_click() {
                answered_above = true;
            }
            // **Reported from use: editing one field, then clicking a
            // different one to edit it next, took two clicks** — the first
            // only closed the one that was open, and a second click (after
            // retyping `edittext`, since picking a run used to be one-shot)
            // was needed to open the next. The click that just closed the
            // old editor is unambiguous about what it wants next: whatever
            // is actually under it. Re-armed and resolved right here, at
            // this same point, rather than left for a click that will not
            // come again on its own.
            if self.tab_mut().editing_run.is_none() {
                if self.errors_said != errors_before {
                    // **Not when the apply was refused.** Whatever the pick
                    // under the click said next would cover the refusal, and
                    // the person would see nothing — the refusal is the
                    // answer to this gesture. The tool is put back in hand
                    // quietly, for the next click.
                    self.arm_tool_without_saying(Tool::PickText, page);
                    answered_above = true;
                } else if let Some(spot) = response.interact_pointer_pos() {
                    let at = view.to_page(spot);
                    self.arm_tool(Tool::PickText, page);
                    if let Some(p) = self.tab_mut().tool.as_mut() {
                        p.points.push(at);
                    }
                    self.resolve_tool();
                    answered_above = true;
                }
            }
        }

        self.show_page_context_menu(ui, &response, page, view);

        let Some(pointer) = response.interact_pointer_pos().or_else(|| response.hover_pos()) else {
            self.tab_mut().last_snap = None;
            return;
        };
        let mut at = view.to_page(pointer);

        // A paste picked up with ⌘V owns the pointer until it is put down: the
        // click that does it is not also a pick, a selection or a mark.
        if self.paste_ghost.is_some() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Crosshair);
            if response.clicked() {
                self.place_paste_ghost(page, at);
            }
            return;
        }

        // Snap, then ortho, then grid — in that order, because a snap is an
        // explicit request for a specific point and must not then be nudged off
        // it by a constraint.
        self.tab_mut().last_snap = None;
        let snapping = self.tab().tool.as_ref().is_some_and(|p| p.kind.wants_snapping());
        let first_point = self.tab().tool.as_ref().and_then(|p| p.points.first().copied());
        if let Some(layer) = self.tab().markup.existing(page).filter(|_| snapping) {
            let radius = HIT_TOLERANCE_PT * 3.0;
            if let Some(snapped) = tools::snap_at(layer, at, radius, self.snaps, None, first_point) {
                at = snapped.at;
                self.tab_mut().last_snap = Some(snapped);
            }
        }
        if snapping && self.tab_mut().last_snap.is_none() {
            if self.ortho {
                if let Some(anchor) = self.tab_mut().tool.as_ref().and_then(|p| p.points.last().copied()) {
                    at = tools::orthogonal(anchor, at);
                }
            }
            if self.grid_pt > 0.0 {
                at = tools::to_grid(at, self.grid_pt);
            }
        }

        if let Some(snapped) = &self.tab_mut().last_snap {
            overlay::draw_snap(ui.painter(), view.to_screen(snapped.at), snapped.kind, theme::snap());
        }

        // A placed-but-unapplied signature, picked with **no tool pending** —
        // before the object tool's own check. Unlike the object tool, this
        // only takes the gesture when it actually found something to do with
        // it — a click on bare paper or on text still reaches the ordinary
        // handling below.
        //
        // **Also with the object tool in hand.** A signature and a placed
        // picture are annotations, not page content, so the object tool's
        // own hit-testing — which walks the page's content objects — cannot
        // see either, and Edit Object is where somebody goes to move a
        // picture they have just put down. Reported from use: after
        // `addimage`, Edit Object could neither select nor move it. Any
        // *other* pending tool still owns the pointer outright.
        if self.tab_mut().tool.is_none() && self.interact_signatures(ui, &response, page, at, view) {
            return;
        }

        // The same, for a plain placed picture — see `interact_placed_images`.
        if self.tab_mut().tool.is_none() && self.interact_placed_images(ui, &response, page, at, view) {
            return;
        }

        // The object tool takes the pointer whole while it is in hand — its
        // clicks select and its drags move or resize, none of which is a mark
        // or a text selection.
        if self.tab_mut().object_tool.is_some() {
            self.interact_objects(ui, &response, page, at, view);
            return;
        }

        // A click is *not* subject to the focus guard, and conflating the two
        // was a bug you could feel: every first click on the page was swallowed
        // to surrender focus, so every tool needed clicking twice to start.
        //
        // The guard exists so that *typing* cannot reach the document — Enter
        // and Delete fired while a number is being entered into a field. A
        // click on the canvas is an unambiguous pointer act aimed at the page,
        // and it also happens to be how someone leaves the command box.
        if response.clicked() {
            ui.memory_mut(|m| m.surrender_focus(command_id));
        }

        // Hand takes the drag before anything else looks at it: in this mode a
        // drag is a scroll, and must not also select.
        //
        // The cursor says so. A lit button in a ribbon of two hundred is not
        // enough of a signal for a mode that stops selection working: the
        // symptom is "text is not selectable any more", which reads as a broken
        // program rather than as the wrong tool being held.
        // The middle button pans whatever tool is held. That is the CAD
        // convention and it costs nothing to honour: it is a button no other
        // gesture here uses, so panning never has to be *selected*, and a tool
        // in progress is not disturbed by moving the paper under it.
        let middle = response.dragged_by(egui::PointerButton::Middle);
        if middle {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
            self.pan(response.drag_delta());
            return;
        }

        if self.tab_mut().pointer == pagify_shell::verbs::PointerMode::Pan {
            ui.output_mut(|o| {
                o.cursor_icon = if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                }
            });
            if response.dragged() {
                self.pan(response.drag_delta());
            }
            self.tab_mut().text_drag = None;
            self.tab_mut().drag_from = None;
            if !response.clicked() {
                return;
            }
        }

        // A pointing hand over a link, the same signal every browser gives —
        // checked ahead of the text cursor below so a link drawn over
        // running text still reads as clickable rather than as selectable
        // prose.
        self.update_pointer_cursor(ui, page, at);
        if self.armed_tool_gesture(&response, at, answered_above) {
            return;
        }
        self.page_drag(ui, &response, view, page, at);
    }

    /// A pointing hand over a link, a text cursor over words — the same
    /// signals a browser gives, checked in that order so a link drawn over
    /// running text still reads as clickable rather than selectable prose.
    fn update_pointer_cursor(&mut self, ui: &mut egui::Ui, page: usize, at: AppPoint) {
        // A selection-resolved tool (Markup/Link/MatchProperties) leaves the
        // pointer behaving exactly as if nothing were armed here — it takes
        // the whole gesture only once a drag actually starts, not while
        // just hovering.
        let no_click_tool_armed = self.tab_mut().tool.as_ref().map_or(true, |t| t.kind.wants_selection());
        let hovering_link = no_click_tool_armed
            && (self.foreign_at(page, at).is_some_and(|n| self.link_uri_at(page, n).is_some())
                || self.internal_link_at(page, at).is_some());
        if hovering_link {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        } else if no_click_tool_armed
            // A text cursor wherever there is text under the pointer, which is
            // the other half of the same answer: on a page whose words are
            // drawn as outlines the cursor stays an arrow, and the reason
            // selection does nothing is visible before the drag rather than
            // after it.
            && self
                .characters(page)
                .and_then(|chars| chars.hit(at.x as f32, at.y as f32))
                .is_some()
        {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Text);
        }
    }

    /// The armed tool takes the whole gesture — see the comment this moved
    /// away from in `interact` for why a wandered click still counts.
    fn armed_tool_gesture(
        &mut self,
        response: &egui::Response,
        at: AppPoint,
        answered_above: bool,
    ) -> bool {
        if self.tab_mut().tool.is_none() {
            return false;
        }
        // Except a selection-resolved tool (Markup/Link/MatchProperties,
        // see `Tool::wants_selection`): those are not waiting for a click
        // or a point at all, they are waiting for an ordinary text-selection
        // drag, the same gesture `page_drag` already handles when no tool
        // is armed. Taking the whole gesture here the way a point/object
        // tool does would swallow that drag as a wandered-click pick instead.
        if self.tab_mut().tool.as_ref().is_some_and(|t| !t.kind.wants_selection()) {
            self.tab_mut().text_drag = None;
            // Not the click the click-away block above has already answered —
            // see `answered_above`.
            if response.drag_started() && !answered_above {
                self.tab_mut().drag_from = Some(at);
            }
            if response.drag_stopped() {
                if let Some(from) = self.tab_mut().drag_from.take() {
                    let wandered =
                        (from.x - at.x).abs().max((from.y - at.y).abs()) <= HIT_TOLERANCE_PT;
                    if wandered {
                        self.take_pick(at);
                    }
                }
            }
            if response.clicked() && !answered_above {
                self.take_pick(at);
            }
            return true;
        }
        false
    }

    /// Everything a drag on the page does when no armed tool owns it: the
    /// rotate handle, a markup grab, a text selection or a marquee — see the
    /// comments this moved away from in `interact`.
    fn page_drag(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        view: PageView,
        page: usize,
        at: AppPoint,
    ) {
        if response.drag_started() {
            // The rotate handle, if a markup shape is already selected here,
            // wins over everything else a drag could mean at this point —
            // the same priority [`Self::signature_handle_at`] gets over a
            // fresh pick. Checked before the text/shape split below so a
            // handle sitting just above a line of text is never mistaken
            // for the start of a text selection.
            let on_rotate_handle = self
                .markup_selection_bounds(page)
                .is_some_and(|bounds| {
                    (view.to_screen(at) - Self::rotate_handle_screen_pos(&bounds, view)).length()
                        <= ROTATE_HANDLE_PX + 2.0
                });

            // A drag that begins **on a character** selects text; one that
            // begins on empty paper selects marks. That is the rule every PDF
            // reader already teaches, and it needs no mode switch — which
            // matters, because a mode the user has to remember is a mode they
            // will be in the wrong one of.
            let on_text = self
                .characters(page)
                .and_then(|chars| chars.hit(at.x as f32, at.y as f32))
                .is_some();

            if on_rotate_handle {
                self.tab_mut().markup_grab = Some(Grab { handle: Some(Handle::Rotate), from: at, by: (0.0, 0.0) });
                self.tab_mut().text_drag = None;
                self.tab_mut().drag_from = None;
            } else if on_text {
                self.tab_mut().text_drag = Some(at);
                self.tab_mut().text_selection = None;
                self.tab_mut().drag_from = None;
                self.tab_mut().markup_grab = None;
            } else {
                // **Reported from use: a drawn shape could only ever be
                // moved by typing `move` and clicking twice.** A drag
                // starting on one of the markup layer's own shapes now
                // picks it up the same way every other kind of object on
                // this page already can — see `finish_markup_grab`. One
                // starting on bare paper still opens the marquee it always
                // has.
                let height = view_height(self, page);
                let hit = self.tab_mut().markup.page(page, height).hit(at, HIT_TOLERANCE_PT * 3.0);
                match hit {
                    Some(index) => {
                        let shift = ui.input(|i| i.modifiers.shift);
                        let layer = self.tab_mut().markup.page(page, height);
                        if !layer.selection().contains(&index) {
                            layer.select_at(at, HIT_TOLERANCE_PT * 3.0, shift);
                        }
                        self.tab_mut().markup_grab = Some(Grab { handle: None, from: at, by: (0.0, 0.0) });
                        self.tab_mut().drag_from = None;
                    }
                    None => {
                        self.tab_mut().drag_from = Some(at);
                        self.tab_mut().markup_grab = None;
                    }
                }
                self.tab_mut().text_drag = None;
            }
        }

        if response.dragged() {
            if let Some(from) = self.tab_mut().text_drag {
                self.tab_mut().text_selection = self
                    .characters(page)
                    .and_then(|chars| {
                        chars.range_between(
                            (from.x as f32, from.y as f32),
                            (at.x as f32, at.y as f32),
                        )
                    });
                if self.tab_mut().text_selection.is_some() {
                    self.tab_mut().selection_page = page;
                }
            }
            if let Some(grab) = self.tab_mut().markup_grab.as_mut() {
                grab.by = ((at.x - grab.from.x) as f32, (at.y - grab.from.y) as f32);
                let handle = grab.handle;
                ui.output_mut(|o| {
                    o.cursor_icon = handle.map(|h| h.cursor()).unwrap_or(egui::CursorIcon::Grabbing)
                });
            }
        }

        if response.drag_stopped() {
            let selecting = self.tab_mut().text_drag.is_some() && self.tab_mut().text_selection.is_some();
            // What a completed selection means to whichever of
            // `Markup`/`Link`/`MatchProperties` is armed (every other kind
            // is untouched by a text-selection drag) is `Tool::on_pointer`
            // now — `Pagify-Phase2-BigTasks.md` §2.3 step 5.
            if selecting {
                if let Some(kind) = self.tab_mut().tool.as_ref().map(|t| t.kind.clone()) {
                    match kind.on_pointer(self) {
                        ToolEffect::None => {}
                        ToolEffect::Say(Kind::Error, text) => self.say_error(text),
                        ToolEffect::Say(_, text) => self.say_info(text),
                        other => unreachable!("on_pointer only returns None or Say, got {other:?}"),
                    }
                }
            }
            self.tab_mut().text_drag = None;
            if let Some(grab) = self.tab_mut().markup_grab.take() {
                if grab.handle == Some(Handle::Rotate) {
                    if let Some(bounds) = self.markup_selection_bounds(page) {
                        self.finish_markup_rotate(page, grab, bounds);
                    }
                } else {
                    self.finish_markup_grab(page, grab);
                }
            } else if let Some(from) = self.tab_mut().drag_from.take() {
                let height = view_height(self, page);
                let layer = self.tab_mut().markup.page(page, height);
                if (from.x - at.x).abs() > 2.0 || (from.y - at.y).abs() > 2.0 {
                    let n = layer.select_box(from, at, ui.input(|i| i.modifiers.shift));
                    self.say_info(format!("{n} selected."));
                }
            }
        }

        if response.clicked() {
            if self.tab_mut().tool.as_ref().is_some_and(|t| !t.kind.wants_selection()) {
                self.take_pick(at);
            } else if let Some(target) = self.internal_link_at(page, at) {
                self.act(Verb::Page(PageTarget::Number(target + 1)));
            } else if let Some(n) = self.foreign_at(page, at) {
                // A link is not "a mark this program did not make" the way
                // the message below means it — clicking one is expected to
                // do the one thing a link is for, not to name it as an
                // annotation somebody might want off the page.
                if let Some(uri) = self.link_uri_at(page, n) {
                    self.open_or_report_link(n, &uri);
                    return;
                }
                // A mark this program did not make. It cannot be reshaped —
                // that would mean reconstructing geometry nobody recorded — but
                // it can be named and taken away, which is the difference
                // between a document you can work on and one you can only look
                // at.
                self.say_info(format!(
                    "annotation {n} on this page — `removemark {n}` takes it off."
                ));
            } else {
                self.tab_mut().text_selection = None;
                let height = view_height(self, page);
                let shift = ui.input(|i| i.modifiers.shift);
                let layer = self.tab_mut().markup.page(page, height);
                layer.select_at(at, HIT_TOLERANCE_PT * 3.0, shift);
            }
        }
    }


    /// The right-click menu, offered wherever the pointer is on the page —
    /// including on frames where the pointer has left the page widget, which
    /// is why it is declared before `interact`'s own early return.
    fn show_page_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        page: usize,
        view: PageView,
    ) {
        // Right-click, which is where a reader looks for Copy first.
        //
        // **Before the early return, not after it.** Moving the pointer towards
        // the menu takes it off the page, so the page stops being hovered, so
        // the function returned before re-declaring the menu — and the menu
        // vanished as you reached for it. A popup has to be offered on every
        // frame it is open, including the frames where the pointer has left the
        // widget that opened it.
        // What the pointer is over, kept before the menu opens: a right-click
        // is a press and a release, and the menu is built on a later frame than
        // the one that knew where the pointer was.
        if response.secondary_clicked() {
            if let Some(spot) = response.interact_pointer_pos() {
                let at = view.to_page(spot);
                self.tab_mut().selected_image = self
                    .images_on(page)
                    .into_iter()
                    .find(|i| {
                        at.x >= i.rect.left as f64
                            && at.x <= i.rect.right as f64
                            && at.y >= i.rect.top as f64
                            && at.y <= i.rect.bottom as f64
                    })
                    .map(|i| (page, i));
                // Where the pointer was, kept for the menu built on a later
                // frame — the same reason `selected_image` is kept.
                self.tab_mut().right_clicked_at = Some((page, at));
                // See `right_click_text_actions`'s own doc: computed once,
                // here, rather than by the menu on every frame it is open.
                self.tab_mut().right_click_text_actions = Some(self.compute_right_click_text_actions(page, at));
            }
        }

        let over_text = self.tab_mut().text_selection.is_some() && page == self.tab_mut().selection_page;
        let over_image = self.tab_mut().selected_image.as_ref().is_some_and(|(p, _)| *p == page);
        // **Offered wherever the pointer is**, not only over a selection.
        //
        // It used to appear only over selected text or a picture, so a
        // right-click on a panel, a rule or bare paper produced nothing at all
        // — which is where somebody whose picture has gone behind something is
        // most likely to be clicking. Reported from use as the layer option not
        // being there.
        if self.tab_mut().doc.is_some() {
            response.context_menu(|ui| {
                // A link under the right-click gets its own two actions,
                // ahead of everything else here — asking whether to follow
                // it or take it off is what a link's own menu is for, and
                // "wherever the pointer is" (see below) already means a link
                // is reached the same way any other page content is.
                self.context_menu_link(ui, page);
                self.context_menu_text(ui, page, over_text);
                self.context_menu_protect(ui, over_text, over_image);
                self.context_menu_layers(ui, page);
                let shown = self.show_layers;
                if ui
                    .button(if shown { "Hide the layer list" } else { "Show all layers" })
                    .clicked()
                {
                    self.show_layers = !shown;
                    if self.show_layers {
                        self.forget_layers();
                    }
                    ui.close();
                }
            });
        }
    }
    /// The link half of the page context menu: open or remove the link under
    /// the pointer. Moved out of `show_page_context_menu` whole.
    fn context_menu_link(&mut self, ui: &mut egui::Ui, page: usize) {
                let link_here = self.tab_mut()
                    .right_clicked_at
                    .filter(|(p, _)| *p == page)
                    .and_then(|(_, at)| self.foreign_at(page, at))
                    .and_then(|n| self.link_uri_at(page, n).map(|uri| (n, uri)));
                if let Some((n, uri)) = link_here {
                    if ui.button(format!("Open {}", short(&uri))).clicked() {
                        self.open_or_report_link(n, &uri);
                        ui.close();
                    }
                    if ui.button("Remove the link").clicked() {
                        self.remove_mark(n);
                        ui.close();
                    }
                    ui.separator();
                }
    }

    /// The text half of the page context menu: Copy, Join into one paragraph
    /// and Split the joined text. Moved out whole.
    fn context_menu_text(&mut self, ui: &mut egui::Ui, page: usize, over_text: bool) {
                if over_text && ui.button("Copy").clicked() {
                    self.tab_mut().copy_wanted = true;
                    ui.close();
                }

                // Read from the cache the click itself filled in — see
                // `right_click_text_actions`'s own doc for why this menu
                // must never recompute these on its own account: it is
                // rebuilt on every repaint of an open popup.
                let actions_here = self.tab_mut()
                    .right_clicked_at
                    .filter(|(p, _)| *p == page)
                    .and_then(|_| self.tab_mut().right_click_text_actions);

                // A selection spanning more than one line or block can be
                // declared one paragraph — see `join_selected_text`'s own
                // doc for why this exists alongside the automatic
                // heuristic rather than instead of it.
                //
                // **Shown disabled, not hidden, when it does not apply** —
                // the same "Choose one above first" shape the layer buttons
                // below already use. Reported from use: hiding it outright
                // whenever the selection was too small to qualify made the
                // feature itself unfindable — a selection covering only one
                // run never showed so much as a hint that joining needed a
                // bigger one.
                if over_text {
                    let joinable = actions_here.is_some_and(|a| a.joinable);
                    if ui.add_enabled(joinable, egui::Button::new("Join into one paragraph")).clicked() {
                        match self.join_selected_text() {
                            Ok(message) => self.say_info(message),
                            Err(e) => self.say_error(e),
                        }
                        ui.close();
                    }
                    if !joinable {
                        ui.small("Select text spanning more than one line or block first.");
                    }
                }
                // The other half of the same feature: undeclaring a join,
                // wherever the right-click landed on one of its runs —
                // not gated on a selection, since splitting one back apart
                // is done by pointing at it, not by selecting it first.
                let split_here = actions_here.and_then(|a| a.split_object);
                if let Some(object) = split_here {
                    if ui.button("Split the joined text").clicked() {
                        self.split_group(page, object);
                        self.say_info("split — these lines are edited on their own again.");
                        ui.close();
                    }
                }
                // Locking is a Protect operation, so it is offered where the
                // Protect tools are rather than on every tab — the same reason
                // the ribbon has tabs at all.
    }

    /// The Protect-tab half of the page context menu: lock the image or the
    /// selection. Moved out whole.
    fn context_menu_protect(&mut self, ui: &mut egui::Ui, over_text: bool, over_image: bool) {
                if self.tab_mut().ribbon == Tab::Protect {
                    if over_image {
                        if ui.button("🔒 Lock this image").clicked() {
                            if let Some((page, image)) = self.tab_mut().selected_image.clone() {
                                self.ask_or_reuse_passcode(
                                    Awaiting::LockImage { page, object: image.object },
                                    "type a passcode to lock this image with, or Escape to give up.",
                                );
                            }
                            ui.close();
                        }
                    }
                    if over_text && ui.button("🔒 Lock the selection").clicked() {
                        self.lock_selection();
                        ui.close();
                    }
                }

                // **The drawing order, where somebody looks for it.** A picture
                // that has gone behind a panel is not reachable from a ribbon
                // button, because the thing to act on is the thing under the
                // pointer.
                ui.separator();
    }

    /// The layer half of the page context menu: the lock badge line, the
    /// topmost-first layer list and the stacking buttons. Moved out whole.
    fn context_menu_layers(&mut self, ui: &mut egui::Ui, page: usize) {
                let spot = self.tab_mut().right_clicked_at.filter(|(p, _)| *p == page).map(|(_, at)| at);
                if let Some(at) = spot {
                    // **A lock badge is not a layer, and says so.** The
                    // chequerboard over a locked picture reads as a grey panel,
                    // and somebody trying to send it back was pointing at the
                    // one thing on the page the drawing order cannot touch.
                    let badge = self.locked_items_on(page).into_iter().find(|i| {
                        at.x >= i.rect.left as f64
                            && at.x <= i.rect.right as f64
                            && at.y >= i.rect.top as f64
                            && at.y <= i.rect.bottom as f64
                    });
                    if let Some(badge) = badge {
                        ui.weak(if badge.is_area {
                            "🔒 Locked words — not a layer. The padlock brings them back."
                        } else if badge.stale {
                            "🔒 A lock badge over a picture that was never taken off — `repairlocks` finishes it."
                        } else {
                            "🔒 A locked picture — not a layer. The padlock brings it back."
                        });
                        ui.separator();
                    }
                    let under = self.layers_under(page, at);
                    if under.is_empty() {
                        ui.weak("Nothing is drawn here.");
                    } else {
                        ui.weak("Layers here — topmost first");
                        let listed: Vec<(usize, String, bool)> = under
                            .iter()
                            .take(8)
                            .filter_map(|index| {
                                self.tab_mut().doc.as_ref()
                                    .and_then(|d| d.caches.layers.as_ref())
                                    .and_then(|(_, l)| l.get(*index))
                                    .map(|d| {
                                        (
                                            *index,
                                            format!("{}  {}", d.kind.describe(), d.label),
                                            !d.movable,
                                        )
                                    })
                            })
                            .collect();
                        for (index, label, grouped) in listed {
                            let picked = self.tab_mut().picked_layer == Some(index);
                            let row = ui.selectable_label(
                                picked,
                                if grouped { format!("{label}   (in a group)") } else { label },
                            );
                            if row.clicked() {
                                self.pick_layer(page, index);
                                ui.close();
                            }
                        }
                        ui.separator();
                        // These act on what has been picked, so somebody can
                        // choose the thing that is *behind* and raise that,
                        // rather than the thing on top of it.
                        let armed = self.tab_mut().picked_layer.is_some();
                        for (label, to) in [
                            ("\u{E5D8}  Move the picked one up", pdf_core::document::Stacking::Up),
                            ("\u{E5DB}  Move the picked one down", pdf_core::document::Stacking::Down),
                            ("\u{E883}  Bring the picked one to front", pdf_core::document::Stacking::Front),
                            ("\u{E882}  Send the picked one to back", pdf_core::document::Stacking::Back),
                        ] {
                            if ui.add_enabled(armed, egui::Button::new(label)).clicked() {
                                self.restack_picked(to);
                                ui.close();
                            }
                        }
                        if !armed {
                            ui.small("Choose one above first.");
                        }
                    }
                    ui.separator();
                }
    }



}
