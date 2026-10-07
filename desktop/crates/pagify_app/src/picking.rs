//! Arming a tool, resolving a pick once it has what it wants, and the click
//! that leaves an open run editor.
//!
//! **Still in its current shape, not redesigned** — see [`crate::pending`]'s
//! own doc for why. `resolve_tool` is still one large dispatch match, not
//! the `Tool::on_click` that is supposed to replace it.

use crate::{area_between, ArmedTool, Awaiting, DrawKind, PendingArticleBox, Tool};
use pagify_shell::markup::HIT_TOLERANCE_PT;
use pagify_shell::page_space::AppPoint;
use pagify_shell::verbs::MeasureKind;
use pagify_shell::{measure, tools};
use pagify_shell::measure::Calibration;

impl crate::PagifyApp {
    pub(crate) fn arm_tool(&mut self, tool: Tool, page: usize) {
        self.arm_tool_without_saying(tool, page);
        let prompt = self.tab().tool.as_ref().map(|t| t.kind.prompt(0, 0));
        if let Some(prompt) = prompt {
            self.say_info(prompt);
        }
    }

    /// [`Self::arm_tool`] without the announcement — for putting a
    /// repeating tool back in hand *after something the person has to
    /// read*. `resolve_tool` uses it to re-arm quietly after a failure,
    /// so the error stays the last thing said instead of being covered by
    /// the tool's own prompt in the same frame (reported from use: a
    /// click that found no text said so, and the tool re-armed itself and
    /// said "click the words to change" over it in the same frame, so the
    /// error only ever showed in the history, which is folded away by
    /// default).
    pub(crate) fn arm_tool_without_saying(&mut self, tool: Tool, page: usize) {
        // The object tool and an armed `Tool` are mutually exclusive — see
        // `take_up_object_tool`'s own clearing of `self.tab_mut().tool` for
        // the other direction. **Reported from use, with a screenshot:
        // after using Edit Object, arming Edit Text left both ribbon
        // buttons lit at once.** Worse than the cosmetic double-highlight:
        // every click kept reaching the object tool's own click-to-select
        // instead of resolving the pick this was arming, because
        // `interact` checks `self.tab_mut().object_tool.is_some()` first
        // and takes the pointer outright when it is.
        if self.tab_mut().object_tool.take().is_some() {
            self.tab_mut().selected = None;
            self.tab_mut().grab = None;
            self.tab_mut().group = Vec::new();
            self.tab_mut().marquee = None;
            self.tab_mut().group_grab = None;
        }
        self.put_down_page_editors("armed a different tool");
        self.tab_mut().tool = Some(ArmedTool { kind: tool, page, objects: Vec::new(), points: Vec::new() });
    }

    /// A click on the page away from the open editor: **applies what was typed**,
    /// except that an edit the engine has already refused, and that has not
    /// changed since, is let go instead of being asked again. Without that a refused
    /// editor would be a trap: every click away would apply it, be refused, and
    /// bring it back, with only Escape — which discards — to get out. The words go
    /// to the clipboard, so letting go loses nothing.
    ///
    /// Returns whether the editor is still open afterwards, so that the click is
    /// not also taken as a pick of whatever is under it.
    pub(crate) fn leave_editor_by_click(&mut self) -> bool {
        let already_refused = self
            .tab()
            .editing_run
            .as_ref()
            .is_some_and(|edit| edit.refusal.as_ref().is_some_and(|refusal| refusal.matches(edit)));
        if already_refused {
            if let Some(edit) = self.tab_mut().editing_run.take() {
                self.offer_typed_text(&edit);
                // The reason was said when it was refused, and is not said again:
                // it can hold the very words that were typed, and the log is not
                // the place for them.
                self.say_info(
                    "let go of the edit the engine refused — nothing was changed, and what was typed is on the clipboard.",
                );
            }
            return false;
        }
        self.apply_editing_page();
        self.tab().editing_run.is_some()
    }

    // -- picks --------------------------------------------------------------

    pub(crate) fn take_pick(&mut self, at: AppPoint) {
        if let Some(armed) = self.tab_mut().tool.as_ref() {
            let page = armed.page;
            let wants_object = armed.objects.len() < armed.kind.wants_objects();
            if wants_object {
                // A generous tolerance: you are aiming at a line with a
                // mouse, and a miss here costs the whole operation.
                let hit = self.tab_mut()
                    .markup
                    .existing(page)
                    .and_then(|layer| layer.hit(at, HIT_TOLERANCE_PT * 3.0));
                match hit {
                    Some(index) => {
                        if let Some(armed) = self.tab_mut().tool.as_mut() {
                            armed.objects.push((index, at));
                        }
                    }
                    // Quietest possible miss: `tool` is not touched at all,
                    // not even to record the attempt.
                    None => {
                        self.say_info("nothing there — click on a mark.");
                        return;
                    }
                }
            } else if let Some(armed) = self.tab_mut().tool.as_mut() {
                armed.points.push(at);
            }
            let ready = self.tab_mut().tool.as_ref().is_some_and(|t| {
                t.objects.len() >= t.kind.wants_objects() && t.points.len() >= t.kind.wants_points()
            });
            if ready {
                self.resolve_tool();
            } else if let Some(armed) = self.tab_mut().tool.as_ref() {
                let prompt = armed.kind.prompt(armed.objects.len(), armed.points.len());
                self.say_info(prompt);
            }
        }
    }

    /// Carries out whatever `take_pick` has finished collecting objects and
    /// points for. Matches on `&armed.kind` rather than taking it, because a
    /// repeating kind needs its own `kind` intact afterward, to re-arm with.
    pub(crate) fn resolve_tool(&mut self) {
        let Some(armed) = self.tab_mut().tool.take() else { return };
        let page = armed.page;
        let height = self.tab_mut()
            .doc
            .as_ref()
            .and_then(|d| d.strip.size_of(page))
            .map(|(_, h)| h as f64)
            .unwrap_or(792.0);
        let outcome = match &armed.kind {
            Tool::Signature => match armed.points.first().copied() {
                Some(at) => self.place_signature(page, at),
                None => Err("signature: nowhere was clicked.".into()),
            },
            Tool::PlaceImage { rgba, width, height } => {
                let (rgba, width, height) = (rgba.clone(), *width, *height);
                match armed.points.first().copied() {
                    Some(at) => self.place_image_at(page, at, rgba, width, height),
                    None => Err("nowhere to place the picture.".into()),
                }
            }
            Tool::PlaceText => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => self.begin_text_box(page, *a, *b),
                _ => Err("text: two corners are needed.".into()),
            },
            Tool::Calibrate { distance, unit } => {
                let (distance, unit) = (*distance, unit.clone());
                match (armed.points.first(), armed.points.get(1)) {
                    (Some(a), Some(b)) => match Calibration::from_two_points(*a, *b, distance, &unit) {
                        Ok(calibration) => {
                            self.tab_mut().calibration = calibration;
                            Ok(self.tab_mut().calibration.describe())
                        }
                        Err(e) => Err(e),
                    },
                    _ => Err("calibrate: two points are needed.".into()),
                }
            }
            Tool::Fill(mark) => {
                let mark = *mark;
                match armed.points.first().copied() {
                    Some(at) => self.stamp_mark(page, mark, at),
                    None => Err("fill: nowhere was clicked.".into()),
                }
            }
            Tool::Redact => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => self.redact(page, *a, *b),
                _ => Err("redact: two corners are needed.".into()),
            },
            Tool::Whiteout => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => self.whiteout(page, *a, *b),
                _ => Err("whiteout: two corners are needed.".into()),
            },
            Tool::SignRectangle => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => self.stamp_box(page, *a, *b),
                _ => Err("rectangle: two corners are needed.".into()),
            },
            Tool::SignLine => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => self.stamp_line(page, *a, *b),
                _ => Err("line: two ends are needed.".into()),
            },
            Tool::Write(text) => {
                let text = text.clone();
                match armed.points.first().copied() {
                    Some(at) => self.write_text_at(page, at, &text),
                    None => Err("nowhere to write.".into()),
                }
            }
            Tool::Draw(kind) => {
                let draw_fill = self.draw_fill;
                let layer = self.tab_mut().markup.page(page, height);
                layer.begin("draw");
                let space = layer.space();
                let p: Vec<cad_kernel::Vec2> =
                    armed.points.iter().map(|q| space.to_kernel(*q)).collect();

                match kind {
                    DrawKind::Line => {
                        layer.add(cad_kernel::Geom::Line(cad_kernel::Line { a: p[0], b: p[1] }));
                        Ok("line added.".into())
                    }
                    DrawKind::Circle => {
                        let radius = (p[1] - p[0]).len();
                        if radius < 1e-6 {
                            Err("circle: that radius is zero.".into())
                        } else {
                            let index = layer.add(cad_kernel::Geom::Circle(cad_kernel::Circle {
                                center: p[0],
                                radius,
                            }));
                            if draw_fill {
                                layer.set_filled(index, true);
                            }
                            Ok(if draw_fill { "filled circle added." } else { "circle added." }.into())
                        }
                    }
                    DrawKind::Rectangle => {
                        let (a, b) = (p[0], p[1]);
                        let corners = [
                            a,
                            cad_kernel::Vec2::new(b.x, a.y),
                            b,
                            cad_kernel::Vec2::new(a.x, b.y),
                        ];
                        let index = layer.add(cad_kernel::Geom::Polyline(cad_kernel::Polyline {
                            vertices: corners
                                .iter()
                                .map(|v| cad_kernel::PolyVertex { pos: *v, bulge: 0.0 })
                                .collect(),
                            closed: true,
                            widths: Vec::new(),
                        }));
                        if draw_fill {
                            layer.set_filled(index, true);
                        }
                        Ok(if draw_fill { "filled rectangle added." } else { "rectangle added." }.into())
                    }
                    DrawKind::Polyline => {
                        if p.len() < 2 {
                            Err("polyline: needs at least two points.".into())
                        } else {
                            layer.add(cad_kernel::Geom::Polyline(cad_kernel::Polyline {
                                vertices: p
                                    .iter()
                                    .map(|v| cad_kernel::PolyVertex { pos: *v, bulge: 0.0 })
                                    .collect(),
                                closed: false,
                                widths: Vec::new(),
                            }));
                            Ok(format!("polyline of {} points added.", p.len()))
                        }
                    }
                    DrawKind::Arrow => {
                        let index = layer.add(cad_kernel::Geom::Line(cad_kernel::Line {
                            a: p[0],
                            b: p[1],
                        }));
                        layer.set_arrow_ends(index, false, true);
                        Ok("arrow added.".into())
                    }
                    DrawKind::Spline => {
                        // A degree-3 B-spline needs more control points than its
                        // degree, so four is the least that makes a real curve.
                        if p.len() < 4 {
                            Err("spline: needs at least four points.".into())
                        } else {
                            let count = p.len();
                            layer.add(cad_kernel::Geom::Spline(cad_kernel::Spline::new_bspline(
                                3, p,
                            )));
                            Ok(format!("spline of {count} points added."))
                        }
                    }
                }
            }
            Tool::EraseMark => match armed.points.first().copied() {
                Some(at) => self.erase_mark_at(page, at),
                None => Err("nothing was clicked.".into()),
            },
            Tool::Measure(MeasureKind::Distance) => Ok(measure::measure_distance(
                &self.tab_mut().calibration,
                armed.points[0],
                armed.points[1],
            )
            .render()),
            Tool::Measure(MeasureKind::Area) => {
                Ok(measure::measure_area(&self.tab_mut().calibration, &armed.points).render())
            }
            Tool::Modify(pick) => {
                let layer = self.tab_mut().markup.page(page, height);
                let space = layer.space();
                let objects: Vec<(usize, cad_kernel::Vec2)> = armed
                    .objects
                    .iter()
                    .map(|(i, at)| (*i, space.to_kernel(*at)))
                    .collect();
                let points: Vec<cad_kernel::Vec2> =
                    armed.points.iter().map(|q| space.to_kernel(*q)).collect();
                tools::run(layer, pick.op, &objects, &points)
            }
            Tool::Lock => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => match area_between(*a, *b) {
                    Some(area) => {
                        // The passcode is asked for *after* the area is drawn, so
                        // it is typed once and used immediately rather than being
                        // held while the user aims.
                        self.ask_or_reuse_passcode(
                            Awaiting::Lock {
                                page,
                                shapes: vec![area],
                                require_complete: true,
                            },
                            "type a passcode to lock it with, or Escape to give up.",
                        );
                        Ok(String::new())
                    }
                    None => Err("lock: that area has no size.".into()),
                },
                _ => Err("lock: two corners are needed.".into()),
            },
            Tool::ArticleBox => match (armed.points.first(), armed.points.get(1)) {
                (Some(a), Some(b)) => match area_between(*a, *b) {
                    Some(rect) => {
                        self.tab_mut().pending_article_box = Some(PendingArticleBox {
                            page,
                            rect,
                            title: String::new(),
                        });
                        Ok(String::new())
                    }
                    None => Err("article box: that area has no size.".into()),
                },
                _ => Err("article box: two corners are needed.".into()),
            },
            Tool::PickText => match armed.points.first().copied() {
                Some(at) => self.pick_text_run(page, at),
                None => Err("nothing was clicked.".into()),
            },
        };
        // Close the checkpoint a draw opened, and drop it if the draw
        // refused, so a failed draw leaves no half-made step in the undo
        // history.
        if matches!(armed.kind, Tool::Draw(_)) {
            if let Some(layer) = self.tab_mut().markup.existing_mut(page) {
                layer.end();
                if outcome.is_err() {
                    layer.forget_last_step();
                }
            }
        }
        let repeats = armed.kind.repeats();
        let failed = outcome.is_err();
        match outcome {
            Ok(said) => self.say_info(said),
            Err(problem) => self.say_error(problem),
        }
        // Back in hand, ready for the next one, quietly after a failure so
        // the error stays the last thing said — see `arm_tool_without_saying`'s
        // doc.
        if repeats && self.tab_mut().editing_run.is_none() {
            if failed {
                self.arm_tool_without_saying(armed.kind, page);
            } else {
                self.arm_tool(armed.kind, page);
            }
        }
    }
}
