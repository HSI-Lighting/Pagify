//! The **Text Style** block of the properties panel, laid out the way a text
//! editor lays it out: the font, its size and its colour on the first row; bold,
//! italic, underline, strikethrough, superscript and subscript on the second;
//! alignment, indent and outdent on the third; lists; then the spacing controls.
//!
//! **Reported from use:** the panel held a size, a colour, a position and a font
//! button. The position went (a page's words are moved by dragging them, not by
//! typing coordinates) and what a person expects of a text box came in.
//!
//! This only *draws* and says what was asked for. It is given what to show
//! ([`Look`]) and which controls do anything ([`Gates`]), and hands back what the
//! person changed ([`Changes`]) — the editor of an existing run and the box for a
//! new one each decide what that means for themselves, so one layout serves both
//! and neither is rewritten when a control starts to work.
//!
//! A control that does not work yet is drawn greyed, with its reason on hover,
//! rather than left out: the panel keeps its shape as the controls come alive.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::theme;

/// One value for the whole selection, or not; superscript, subscript or neither —
/// the box's own model's types, so the panel shows what the model holds.
pub(crate) use pagify_shell::rich_text::{Mixed, Script};

/// Where lines sit between the left and right of their box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// What the panel shows: the selection's look, each part one value or *mixed*.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Look {
    /// The font's name, as the font button reads.
    pub font: Mixed<String>,
    pub size: Mixed<f32>,
    pub color: Mixed<[u8; 3]>,
    pub bold: Mixed<bool>,
    pub italic: Mixed<bool>,
    pub underline: Mixed<bool>,
    pub strike: Mixed<bool>,
    pub script: Mixed<Script>,
    /// Points after every character.
    pub letter_spacing: Mixed<f32>,
    /// Percent of the letters' own width.
    pub width: Mixed<f32>,
    /// Points up (+) or down (−).
    pub baseline: Mixed<f32>,
    /// `None` when the box has no alignment to show (nothing selected).
    pub align: Option<Align>,
}

/// Which controls do something. `None` is "works"; `Some(reason)` is greyed out,
/// and the reason is what the person is told on hover.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gates {
    pub bold: Option<String>,
    pub italic: Option<String>,
    pub underline: Option<String>,
    pub strike: Option<String>,
    pub script: Option<String>,
    pub left: Option<String>,
    pub center: Option<String>,
    pub right: Option<String>,
    pub justify: Option<String>,
    pub indent: Option<String>,
    pub lists: Option<String>,
    pub letter_spacing: Option<String>,
    pub width: Option<String>,
    pub baseline: Option<String>,
    pub line_spacing: Option<String>,
    pub paragraph_spacing: Option<String>,
}

impl Gates {
    /// Everything the panel holds, switched off for one reason.
    pub(crate) fn all_off(reason: &str) -> Gates {
        let r = || Some(reason.to_string());
        Gates {
            bold: r(),
            italic: r(),
            underline: r(),
            strike: r(),
            script: r(),
            left: r(),
            center: r(),
            right: r(),
            justify: r(),
            indent: r(),
            lists: r(),
            letter_spacing: r(),
            width: r(),
            baseline: r(),
            line_spacing: r(),
            paragraph_spacing: r(),
        }
    }
}

/// What the person changed this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Changes {
    /// The font button was pressed: show the font picker.
    pub font_clicked: bool,
    pub size: Option<f32>,
    pub color: Option<[u8; 3]>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub script: Option<Script>,
    pub align: Option<Align>,
    pub letter_spacing: Option<f32>,
    pub width: Option<f32>,
    pub baseline: Option<f32>,
}

impl Changes {
    pub(crate) fn any(&self) -> bool {
        *self != Changes::default()
    }
}

/// The sizes the size menu offers.
pub(crate) const SIZES: [f32; 19] = [
    6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 60.0, 72.0, 96.0,
];

/// The letter spacings its menu offers, in points.
pub(crate) const LETTER_SPACINGS: [f32; 7] = [-1.0, -0.5, 0.0, 0.5, 1.0, 2.0, 5.0];

/// The character widths its menu offers, in percent.
pub(crate) const WIDTHS: [f32; 6] = [75.0, 90.0, 100.0, 110.0, 125.0, 150.0];

/// The baseline shifts its menu offers, in points.
pub(crate) const BASELINES: [f32; 7] = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0];

const BUTTON: f32 = 30.0;

/// Draw the block. `id` keeps the collapsing header's state apart from any
/// other copy of this panel.
pub(crate) fn show(ui: &mut Ui, id: egui::Id, look: &Look, gates: &Gates) -> Changes {
    let mut changes = Changes::default();
    egui::CollapsingHeader::new(egui::RichText::new("Text Style").color(theme::ink_dim()))
        .id_salt(id)
        .default_open(true)
        .show(ui, |ui| {
            ui.add_space(4.0);
            first_row(ui, look, &mut changes);
            ui.add_space(8.0);
            style_row(ui, look, gates, &mut changes);
            ui.add_space(6.0);
            align_row(ui, look, gates, &mut changes);
            ui.add_space(6.0);
            list_row(ui, gates);
            ui.add_space(8.0);
            spacing_rows(ui, look, gates, &mut changes);
        });
    changes
}

/// The font, its size and its colour, on one row.
///
/// **Laid out from the right, so the font button takes what is *measured* to be
/// left, not what is assumed.** It used to be sized as "the width available,
/// less what the other two were expected to take" — and the colour button took
/// 14 pt more than the 26 expected of it (egui's own button is `interact_size`,
/// 40 pt). A panel is as wide as its content, so the row outgrew the panel by
/// that much on every frame, for as long as the window had room: opening Edit
/// Text gave a Properties panel as wide as the window, which came straight back
/// when dragged narrower (reported from use). A constant that is right today is
/// wrong the day a widget changes; a measurement cannot be.
fn first_row(ui: &mut Ui, look: &Look, changes: &mut Changes) {
    let colour = 26.0;
    let size_box = 74.0;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The colour — a button of the size this row was laid out for. Mixed, it
            // shows grey; a colour picked sets every character.
            let mut rgb = match look.color {
                Mixed::One(rgb) => rgb,
                Mixed::Mixed => [128, 128, 128],
            };
            let picked = ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size = Vec2::splat(colour);
                    ui.color_edit_button_srgb(&mut rgb).on_hover_text("The colour.").changed()
                })
                .inner;
            if picked {
                changes.color = Some(rgb);
            }

            // The size: a number that can be typed or dragged, and a menu of the usual
            // ones. Mixed, the number is left blank and the menu sets every character.
            let (box_rect, _) = ui.allocate_exact_size(Vec2::new(size_box, 26.0), Sense::hover());
            let mut inner = ui.new_child(
                egui::UiBuilder::new().max_rect(box_rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            match look.size {
                Mixed::One(mut size) => {
                    let number = inner.add(
                        egui::DragValue::new(&mut size).speed(0.25).range(1.0..=400.0).max_decimals(1).min_decimals(0),
                    );
                    if number.changed() {
                        changes.size = Some(size);
                    }
                    number.on_hover_text(
                        "The size, in points. On a line of mixed sizes the box sits the smaller words a little low; \
                         the page lines them up.",
                    );
                }
                Mixed::Mixed => inner.add_space(size_box - 22.0),
            }
            let menu = inner.add(egui::Button::new("").min_size(Vec2::new(18.0, 22.0)).frame(false));
            draw_triangle(inner.painter(), menu.rect.center(), theme::ink_dim());
            egui::Popup::menu(&menu).show(|ui| {
                for preset in SIZES {
                    let label = if preset.fract() == 0.0 { format!("{preset:.0}") } else { format!("{preset}") };
                    if ui.selectable_label(look.size == Mixed::One(preset), label).clicked() {
                        changes.size = Some(preset);
                        ui.close();
                    }
                }
            });

            // The font: a wide button that reads as a drop-down and opens the
            // picker, and takes everything that is left.
            let font_width = ui.available_width().max(90.0);
            let font = match &look.font {
                Mixed::One(name) => name.as_str(),
                Mixed::Mixed => "",
            };
            let (rect, response) = ui.allocate_exact_size(Vec2::new(font_width, 26.0), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Font {font}")));
            draw_field(ui.painter(), rect, &response);
            ui.painter().with_clip_rect(rect.shrink2(Vec2::new(8.0, 0.0)).with_max_x(rect.right() - 20.0)).text(
                Pos2::new(rect.left() + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                font,
                egui::FontId::proportional(12.5),
                theme::ink(),
            );
            draw_triangle(ui.painter(), Pos2::new(rect.right() - 11.0, rect.center().y), theme::ink_dim());
            if response.on_hover_text("The font. Click to choose another.").clicked() {
                changes.font_clicked = true;
            }
        });
    });
}

fn style_row(ui: &mut Ui, look: &Look, gates: &Gates, changes: &mut Changes) {
    let spread = spread(ui, 6);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spread;
        // A press turns a toggle off where all of the selection has it, and on
        // otherwise — mixed included, as in a word processor.
        let pressed = |shown: Mixed<bool>| shown != Mixed::One(true);
        if glyph_toggle(ui, "Bold", look.bold, &gates.bold, |p, r, c| draw_letter(p, r, "B", c, true, false, false)).clicked()
            && gates.bold.is_none()
        {
            changes.bold = Some(pressed(look.bold));
        }
        if glyph_toggle(ui, "Italic", look.italic, &gates.italic, |p, r, c| draw_letter(p, r, "I", c, false, true, false))
            .clicked()
            && gates.italic.is_none()
        {
            changes.italic = Some(pressed(look.italic));
        }
        if glyph_toggle(ui, "Underline", look.underline, &gates.underline, |p, r, c| {
            draw_letter(p, r, "U", c, false, false, false);
            let y = r.center().y + 8.5;
            p.line_segment([Pos2::new(r.center().x - 6.0, y), Pos2::new(r.center().x + 6.0, y)], Stroke::new(1.3, c));
        })
        .clicked()
            && gates.underline.is_none()
        {
            changes.underline = Some(pressed(look.underline));
        }
        if glyph_toggle(ui, "Strikethrough", look.strike, &gates.strike, |p, r, c| {
            draw_letter(p, r, "S", c, false, false, false);
            p.line_segment(
                [Pos2::new(r.center().x - 7.0, r.center().y), Pos2::new(r.center().x + 7.0, r.center().y)],
                Stroke::new(1.3, c),
            );
        })
        .clicked()
            && gates.strike.is_none()
        {
            changes.strike = Some(pressed(look.strike));
        }
        let is = |which: Script| match look.script {
            Mixed::One(script) => Mixed::One(script == which),
            Mixed::Mixed => Mixed::Mixed,
        };
        let sup = is(Script::Super);
        if glyph_toggle(ui, "Superscript", sup, &gates.script, |p, r, c| {
            draw_letter(p, r, "T", c, false, false, false);
            p.text(
                Pos2::new(r.center().x + 7.5, r.center().y - 6.5),
                egui::Align2::CENTER_CENTER,
                "1",
                egui::FontId::proportional(8.5),
                c,
            );
        })
        .clicked()
            && gates.script.is_none()
        {
            changes.script = Some(if pressed(sup) { Script::Super } else { Script::Normal });
        }
        let sub = is(Script::Sub);
        if glyph_toggle(ui, "Subscript", sub, &gates.script, |p, r, c| {
            draw_letter(p, r, "T", c, false, false, false);
            p.text(
                Pos2::new(r.center().x + 7.5, r.center().y + 6.5),
                egui::Align2::CENTER_CENTER,
                "1",
                egui::FontId::proportional(8.5),
                c,
            );
        })
        .clicked()
            && gates.script.is_none()
        {
            changes.script = Some(if pressed(sub) { Script::Sub } else { Script::Normal });
        }
    });
}

fn align_row(ui: &mut Ui, look: &Look, gates: &Gates, changes: &mut Changes) {
    let spread = spread(ui, 6);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spread;
        let mut one = |ui: &mut Ui, name: &str, kind: Icon, shown: bool, gate: &Option<String>, set: Option<Align>| {
            if glyph_toggle(ui, name, Mixed::One(shown), gate, |p, r, c| draw_icon(p, r, kind, c)).clicked() && gate.is_none() {
                if let Some(a) = set {
                    changes.align = Some(a);
                }
            }
        };
        one(ui, "Align left", Icon::Left, look.align == Some(Align::Left), &gates.left, Some(Align::Left));
        one(ui, "Align centre", Icon::Center, look.align == Some(Align::Center), &gates.center, Some(Align::Center));
        one(ui, "Align right", Icon::Right, look.align == Some(Align::Right), &gates.right, Some(Align::Right));
        one(ui, "Justify", Icon::Justify, look.align == Some(Align::Justify), &gates.justify, Some(Align::Justify));
        one(ui, "Indent", Icon::Indent, false, &gates.indent, None);
        one(ui, "Outdent", Icon::Outdent, false, &gates.indent, None);
    });
}

fn list_row(ui: &mut Ui, gates: &Gates) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (name, icon) in [("Bulleted list", Icon::Bullets), ("Numbered list", Icon::Numbers)] {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(BUTTON + 14.0, BUTTON), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, gates.lists.is_none(), name));
            let colour = control_colour(&response, false, gates.lists.is_some());
            if response.hovered() && gates.lists.is_none() {
                ui.painter().rect_filled(rect, 3.0, theme::raised());
            }
            draw_icon(ui.painter(), Rect::from_min_size(rect.min, Vec2::splat(BUTTON)), icon, colour);
            draw_triangle(ui.painter(), Pos2::new(rect.right() - 6.0, rect.center().y + 5.0), colour);
            reason(response, name, &gates.lists);
        }
    });
}

fn spacing_rows(ui: &mut Ui, look: &Look, gates: &Gates, changes: &mut Changes) {
    // Two columns, as laid out: spacing between lines and between paragraphs;
    // the width of the letters and where they sit on the line; and the gap
    // between letters.
    ui.horizontal(|ui| {
        spacing_field(ui, Icon::LineSpacing, "Line spacing", "0.00", &gates.line_spacing);
        ui.add_space(8.0);
        spacing_field(ui, Icon::ParagraphSpacing, "Paragraph spacing", "0.00", &gates.paragraph_spacing);
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(v) = number_input(
            ui,
            Icon::CharacterScale,
            "Character width",
            "the box spaces the letters this wide but does not squeeze their shapes; the page draws them exactly.",
            look.width,
            10.0..=400.0,
            &WIDTHS,
            "%",
            &gates.width,
        ) {
            changes.width = Some(v);
        }
        ui.add_space(8.0);
        if let Some(v) = number_input(
            ui,
            Icon::Baseline,
            "Baseline shift",
            "the box draws the letters on their line; the page moves them up or down by this much.",
            look.baseline,
            -50.0..=50.0,
            &BASELINES,
            "",
            &gates.baseline,
        ) {
            changes.baseline = Some(v);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(v) = number_input(
            ui,
            Icon::Tracking,
            "Letter spacing",
            "",
            look.letter_spacing,
            -20.0..=20.0,
            &LETTER_SPACINGS,
            "",
            &gates.letter_spacing,
        ) {
            changes.letter_spacing = Some(v);
        }
    });
    ui.add_space(4.0);
}

/// A number that can be typed or dragged, beside a menu of the usual ones — or,
/// where the selection's characters differ, a blank field with the same menu.
/// Hands back a value set this frame; a closed gate makes it inert. `note` is added
/// to the hover: how near the box's own drawing comes.
#[allow(clippy::too_many_arguments)]
fn number_input(
    ui: &mut Ui,
    icon: Icon,
    name: &str,
    note: &str,
    value: Mixed<f32>,
    range: std::ops::RangeInclusive<f32>,
    presets: &[f32],
    unit: &str,
    gate: &Option<String>,
) -> Option<f32> {
    let disabled = gate.is_some();
    let hover = if note.is_empty() { name.to_string() } else { format!("{name} \u{2014} {note}") };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0 + 78.0, 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, !disabled, name));
    let colour = if disabled { theme::ink_faint() } else { theme::ink() };
    draw_icon(ui.painter(), Rect::from_min_size(rect.min, Vec2::splat(28.0)), icon, colour);
    let field = Rect::from_min_max(Pos2::new(rect.left() + 32.0, rect.top()), rect.max);
    draw_field(ui.painter(), field, &response);

    let mut picked = None;
    if disabled {
        if let Mixed::One(v) = value {
            ui.painter().text(
                Pos2::new(field.left() + 8.0, field.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{v}{unit}"),
                egui::FontId::proportional(12.5),
                colour,
            );
        }
    } else {
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(field.shrink2(Vec2::new(4.0, 2.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        match value {
            Mixed::One(mut v) => {
                let number = inner.add_sized(
                    Vec2::new(50.0, 22.0),
                    egui::DragValue::new(&mut v).speed(0.1).range(range).suffix(unit).max_decimals(2),
                );
                if number.changed() {
                    picked = Some(v);
                }
                number.on_hover_text(&hover);
            }
            Mixed::Mixed => inner.add_space(50.0),
        }
        let (menu_rect, menu) = inner.allocate_exact_size(Vec2::new(16.0, 22.0), Sense::click());
        menu.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name} presets")));
        draw_triangle(inner.painter(), menu_rect.center(), theme::ink_dim());
        egui::Popup::menu(&menu).show(|ui| {
            for preset in presets {
                if ui.selectable_label(value == Mixed::One(*preset), format!("{preset}{unit}")).clicked() {
                    picked = Some(*preset);
                    ui.close();
                }
            }
        });
    }
    reason(response, &hover, gate);
    picked
}

fn spacing_field(ui: &mut Ui, icon: Icon, name: &str, value: &str, gate: &Option<String>) {
    let disabled = gate.is_some();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0 + 78.0, 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !disabled, name));
    let colour = if disabled { theme::ink_faint() } else { theme::ink() };
    draw_icon(ui.painter(), Rect::from_min_size(rect.min, Vec2::splat(28.0)), icon, colour);
    let field = Rect::from_min_max(Pos2::new(rect.left() + 32.0, rect.top()), rect.max);
    draw_field(ui.painter(), field, &response);
    ui.painter().text(
        Pos2::new(field.left() + 8.0, field.center().y),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::proportional(12.5),
        colour,
    );
    draw_triangle(ui.painter(), Pos2::new(field.right() - 11.0, field.center().y), colour);
    reason(response, name, gate);
}

// -- small pieces ------------------------------------------------------------

/// The gap that spreads `n` buttons evenly across the row.
fn spread(ui: &Ui, n: usize) -> f32 {
    ((ui.available_width() - BUTTON * n as f32) / (n.saturating_sub(1)) as f32).clamp(2.0, 18.0)
}

/// What the hover says: the control's name, and when it is greyed, why.
fn reason(response: Response, name: &str, gate: &Option<String>) -> Response {
    match gate {
        Some(why) => response.on_hover_text(format!("{name} \u{2014} {why}")),
        None => response.on_hover_text(name),
    }
}

fn control_colour(response: &Response, selected: bool, disabled: bool) -> Color32 {
    if disabled {
        theme::ink_faint()
    } else if selected {
        Color32::WHITE
    } else if response.hovered() {
        theme::ink()
    } else {
        theme::ink_dim()
    }
}

/// A square toggle with a drawn glyph: lit when all of the selection has it,
/// outlined when only some does. Greyed (and inert) when `gate` is `Some`.
fn glyph_toggle(
    ui: &mut Ui,
    name: &str,
    selected: Mixed<bool>,
    gate: &Option<String>,
    draw: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    let disabled = gate.is_some();
    let lit = selected == Mixed::One(true);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, !disabled, lit, name));
    if lit && !disabled {
        ui.painter().rect_filled(rect, 3.0, theme::violet_deep());
    } else if selected == Mixed::Mixed && !disabled {
        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, theme::violet()), egui::StrokeKind::Inside);
    } else if response.hovered() && !disabled {
        ui.painter().rect_filled(rect, 3.0, theme::raised());
    }
    draw(ui.painter(), rect, control_colour(&response, lit, disabled));
    match (selected, gate) {
        (Mixed::Mixed, None) => response.on_hover_text(format!("{name} \u{2014} mixed")),
        _ => reason(response, name, gate),
    }
}

fn draw_letter(painter: &Painter, rect: Rect, letter: &str, colour: Color32, bold: bool, italic: bool, _underline: bool) {
    // egui has no bold face of its own, so bold is drawn the way a font would
    // thicken a stroke: a second pass a hair to each side. Italic it does have —
    // egui leans the glyphs itself (`TextFormat::italics`). **Not a sheared copy
    // of the galley's mesh:** a galley's UVs are in texels until egui's text
    // tessellator normalises them, so a mesh copied out of one drew nothing.
    let mut job = egui::text::LayoutJob::default();
    job.append(
        letter,
        0.0,
        egui::TextFormat { font_id: egui::FontId::proportional(17.0), color: colour, italics: italic, ..Default::default() },
    );
    let galley = painter.layout_job(job);
    let at = rect.center() - galley.size() / 2.0;
    if bold {
        painter.galley(at + Vec2::new(0.6, 0.0), galley.clone(), colour);
        painter.galley(at - Vec2::new(0.6, 0.0), galley.clone(), colour);
    }
    painter.galley(at, galley, colour);
}

fn draw_field(painter: &Painter, rect: Rect, response: &Response) {
    painter.rect_filled(rect, 3.0, theme::paper());
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, if response.hovered() { theme::violet_bright() } else { theme::ink_faint() }),
        egui::StrokeKind::Inside,
    );
}

fn draw_triangle(painter: &Painter, centre: Pos2, colour: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            centre + Vec2::new(-3.5, -2.0),
            centre + Vec2::new(3.5, -2.0),
            centre + Vec2::new(0.0, 2.5),
        ],
        colour,
        Stroke::NONE,
    ));
}

#[derive(Debug, Clone, Copy)]
enum Icon {
    Left,
    Center,
    Right,
    Justify,
    Indent,
    Outdent,
    Bullets,
    Numbers,
    LineSpacing,
    ParagraphSpacing,
    CharacterScale,
    Baseline,
    Tracking,
}

/// Five rules of lengths that say which way the lines are set.
fn rules(painter: &Painter, rect: Rect, lengths: [f32; 5], colour: Color32, align: Align, inset: f32) {
    let width = 17.0;
    let left = rect.center().x - width / 2.0 + inset;
    let top = rect.center().y - 7.0;
    for (i, fraction) in lengths.iter().enumerate() {
        let w = (width - inset) * fraction;
        let x0 = match align {
            Align::Left | Align::Justify => left,
            Align::Center => rect.center().x + inset / 2.0 - w / 2.0,
            Align::Right => left + (width - inset) - w,
        };
        let y = top + i as f32 * 3.5;
        painter.line_segment([Pos2::new(x0, y), Pos2::new(x0 + w, y)], Stroke::new(1.3, colour));
    }
}

fn draw_icon(painter: &Painter, rect: Rect, icon: Icon, c: Color32) {
    let stroke = Stroke::new(1.3, c);
    match icon {
        Icon::Left => rules(painter, rect, [1.0, 0.6, 1.0, 0.6, 1.0], c, Align::Left, 0.0),
        Icon::Center => rules(painter, rect, [1.0, 0.6, 1.0, 0.6, 1.0], c, Align::Center, 0.0),
        Icon::Right => rules(painter, rect, [1.0, 0.6, 1.0, 0.6, 1.0], c, Align::Right, 0.0),
        Icon::Justify => rules(painter, rect, [1.0; 5], c, Align::Justify, 0.0),
        Icon::Indent | Icon::Outdent => {
            rules(painter, rect, [1.0, 0.7, 0.7, 0.7, 1.0], c, Align::Left, 0.0);
            let y = rect.center().y;
            let x = rect.left() + 5.0;
            let (tip, tail) = if matches!(icon, Icon::Indent) { (x + 5.0, x) } else { (x, x + 5.0) };
            painter.line_segment([Pos2::new(tail, y), Pos2::new(tip, y)], stroke);
            painter.line_segment([Pos2::new(tip, y), Pos2::new(tip + if tip > tail { -2.5 } else { 2.5 }, y - 2.5)], stroke);
            painter.line_segment([Pos2::new(tip, y), Pos2::new(tip + if tip > tail { -2.5 } else { 2.5 }, y + 2.5)], stroke);
        }
        Icon::Bullets | Icon::Numbers => {
            for i in 0..3 {
                let y = rect.center().y - 6.0 + i as f32 * 6.0;
                if matches!(icon, Icon::Bullets) {
                    painter.circle_filled(Pos2::new(rect.left() + 6.0, y), 1.6, c);
                } else {
                    painter.text(
                        Pos2::new(rect.left() + 6.0, y),
                        egui::Align2::CENTER_CENTER,
                        (i + 1).to_string(),
                        egui::FontId::proportional(7.5),
                        c,
                    );
                }
                painter.line_segment([Pos2::new(rect.left() + 11.0, y), Pos2::new(rect.left() + 24.0, y)], stroke);
            }
        }
        Icon::LineSpacing | Icon::ParagraphSpacing => {
            // Rules with an arrow between them, up and down.
            let x = rect.left() + 8.0;
            painter.line_segment([Pos2::new(x, rect.center().y - 7.0), Pos2::new(x, rect.center().y + 7.0)], stroke);
            painter.line_segment([Pos2::new(x - 2.5, rect.center().y - 4.5), Pos2::new(x, rect.center().y - 7.0)], stroke);
            painter.line_segment([Pos2::new(x + 2.5, rect.center().y - 4.5), Pos2::new(x, rect.center().y - 7.0)], stroke);
            painter.line_segment([Pos2::new(x - 2.5, rect.center().y + 4.5), Pos2::new(x, rect.center().y + 7.0)], stroke);
            painter.line_segment([Pos2::new(x + 2.5, rect.center().y + 4.5), Pos2::new(x, rect.center().y + 7.0)], stroke);
            let extra = matches!(icon, Icon::ParagraphSpacing);
            for (i, fraction) in [1.0, 0.7, 1.0].iter().enumerate() {
                let y = rect.center().y - 5.0 + i as f32 * 5.0 + if extra && i == 2 { 2.0 } else { 0.0 };
                painter.line_segment(
                    [Pos2::new(rect.left() + 13.0, y), Pos2::new(rect.left() + 13.0 + 13.0 * fraction, y)],
                    stroke,
                );
            }
        }
        Icon::CharacterScale => {
            // A rule with arrows to the sides, over a line of letters.
            let y = rect.center().y + 4.0;
            painter.line_segment([Pos2::new(rect.left() + 5.0, y), Pos2::new(rect.right() - 5.0, y)], stroke);
            painter.text(rect.center() - Vec2::new(0.0, 4.0), egui::Align2::CENTER_CENTER, "T", egui::FontId::proportional(12.0), c);
        }
        Icon::Baseline => {
            painter.line_segment([Pos2::new(rect.left() + 6.0, rect.center().y + 6.0), Pos2::new(rect.right() - 6.0, rect.center().y + 6.0)], stroke);
            painter.text(rect.center() - Vec2::new(0.0, 2.0), egui::Align2::CENTER_CENTER, "T", egui::FontId::proportional(12.0), c);
        }
        Icon::Tracking => {
            painter.text(rect.center() - Vec2::new(0.0, 3.0), egui::Align2::CENTER_CENTER, "A|B", egui::FontId::proportional(9.0), c);
            painter.line_segment([Pos2::new(rect.left() + 6.0, rect.center().y + 6.0), Pos2::new(rect.right() - 6.0, rect.center().y + 6.0)], stroke);
        }
    }
}

// ---- bold and italic are real faces: the same family's Bold, Italic, … ----

/// A subset-embedded font's name is `ABCDEF+Name`; the tag is not part of it.
fn strip_subset(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

fn name_words(name: &str) -> Vec<String> {
    strip_subset(name)
        .split(|c: char| c == ' ' || c == '-' || c == '_')
        .filter(|w| !w.is_empty())
        .map(|w| without_vendor_mark(&w.to_ascii_lowercase()))
        .collect()
}

/// Monotype's PostScript names end in `MT` — `ArialMT`, `Arial-BoldMT`,
/// `TimesNewRomanPS-BoldMT`, as Windows lists them — a vendor's mark, not part
/// of the family or the style.
fn without_vendor_mark(word: &str) -> String {
    match word.strip_suffix("mt") {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => word.to_string(),
    }
}

/// Words that name a style or a weight rather than a family.
fn is_style_word(word: &str) -> bool {
    word.contains("bold")
        || word.contains("italic")
        || word.contains("oblique")
        || ["regular", "book", "roman", "normal", "thin", "extralight", "ultralight", "light", "medium", "black", "heavy"]
            .contains(&word)
}

/// Whether a font's own name says bold, and whether it says italic.
pub(crate) fn style_of(name: &str) -> (bool, bool) {
    let words = name_words(name);
    (
        words.iter().any(|w| w.contains("bold")),
        words.iter().any(|w| w.contains("italic") || w.contains("oblique")),
    )
}

/// The family a font's name belongs to: its name without the style and weight words.
pub(crate) fn family_of(name: &str) -> String {
    name_words(name).into_iter().filter(|w| !is_style_word(w)).collect::<Vec<_>>().join(" ")
}

/// The face of `current`'s family that is `bold` and `italic` as asked, from
/// `names` (the fonts there are) — `None` when the family has no such face.
/// Compared without case, spaces, hyphens or a vendor's `MT`, so `Arial Bold`,
/// `Arial-Bold`, `ArialBold` and `Arial-BoldMT` are one name.
pub(crate) fn family_variant(current: &str, bold: bool, italic: bool, names: &[&str]) -> Option<String> {
    let key = |s: &str| -> String {
        name_words(s).concat().chars().filter(|c| c.is_alphanumeric()).collect::<String>()
    };
    let family = key(&family_of(current));
    if family.is_empty() {
        return None;
    }
    let styles: &[&str] = match (bold, italic) {
        (false, false) => &["", "regular", "roman", "book", "normal"],
        (true, false) => &["bold"],
        (false, true) => &["italic", "oblique"],
        (true, true) => &["bolditalic", "boldoblique", "italicbold"],
    };
    styles.iter().find_map(|style| {
        let wanted = format!("{family}{style}");
        names.iter().find(|name| key(name) == wanted).map(|name| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look {
            font: Mixed::One("SourceSansPro-Regular".into()),
            size: Mixed::One(14.0),
            color: Mixed::One([0, 0, 0]),
            bold: Mixed::One(false),
            italic: Mixed::One(false),
            underline: Mixed::One(false),
            strike: Mixed::One(false),
            script: Mixed::One(Script::Normal),
            letter_spacing: Mixed::One(0.0),
            width: Mixed::One(100.0),
            baseline: Mixed::One(0.0),
            align: Some(Align::Left),
        }
    }

    /// The panel in a window of its own, keeping the last change it handed back.
    fn panel(look: Look, gates: Gates) -> (egui_kittest::Harness<'static>, std::rc::Rc<std::cell::RefCell<Changes>>) {
        let got = std::rc::Rc::new(std::cell::RefCell::new(Changes::default()));
        let keep = got.clone();
        let mut h = egui_kittest::Harness::new_ui(move |ui| {
            let changes = show(ui, egui::Id::new("style"), &look, &gates);
            if changes.any() {
                *keep.borrow_mut() = changes;
            }
        });
        h.run();
        (h, got)
    }

    #[test]
    fn a_mixed_toggle_is_neither_lit_nor_dark_and_turns_it_on() {
        use egui_kittest::kittest::Queryable;
        let (mut h, got) = panel(Look { underline: Mixed::Mixed, ..look() }, Gates::default());
        let button = h.get_by_label("Underline").rect();
        let near = |r: &egui::Rect| r.center().distance(button.center()) < 1.0;
        let shapes = &h.output().shapes;
        assert!(
            shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if near(&r.rect) && r.stroke.color == theme::violet() && r.fill == Color32::TRANSPARENT)),
            "a mixed toggle is drawn as an outline"
        );
        assert!(
            !shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if near(&r.rect) && r.fill == theme::violet_deep())),
            "a mixed toggle is drawn lit"
        );
        h.get_by_label("Underline").click();
        h.run();
        assert_eq!(got.borrow().underline, Some(true));
    }

    #[test]
    fn the_spacing_inputs_hand_back_what_was_picked() {
        use egui_kittest::kittest::Queryable;
        let (mut h, got) = panel(look(), Gates::default());
        let picks: [(&str, &str, fn(&Changes) -> Option<f32>, f32); 3] = [
            ("Character width presets", "125%", |c| c.width, 125.0),
            ("Letter spacing presets", "2", |c| c.letter_spacing, 2.0),
            ("Baseline shift presets", "-1", |c| c.baseline, -1.0),
        ];
        for (menu, item, read, want) in picks {
            h.get_by_label(menu).click();
            h.run();
            h.get_by_label(item).click();
            h.run();
            assert_eq!(read(&got.borrow()), Some(want), "{menu} → {item}");
        }
    }

    #[test]
    fn a_mixed_number_field_shows_no_number() {
        use egui_kittest::kittest::Queryable;
        let (mut h, _) = panel(Look { letter_spacing: Mixed::Mixed, ..look() }, Gates::default());
        let field = h.get_by_label("Letter spacing").rect();
        let digits = h.output().shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) => field.contains(t.pos) && t.galley.text().chars().any(|c| c.is_ascii_digit()),
            _ => false,
        });
        assert!(!digits, "a mixed field shows a number");
        h.get_by_label("Letter spacing presets").click();
        h.run();
        assert!(h.query_by_label("2").is_some(), "the presets are not offered");
    }

    #[test]
    fn a_gated_spacing_input_is_inert_with_its_reason() {
        use egui_kittest::kittest::Queryable;
        let gates = Gates { letter_spacing: Some("comes with paragraph styles".into()), ..Gates::default() };
        let (mut h, got) = panel(look(), gates);
        assert!(h.query_by_label("Letter spacing presets").is_none(), "a closed input offers its presets");
        h.get_by_label("Letter spacing").click();
        h.run();
        assert_eq!(*got.borrow(), Changes::default(), "a closed input changed something");
        h.get_by_label("Letter spacing").hover();
        h.run_steps(4);
        assert!(h.query_by_label_contains("comes with paragraph styles").is_some(), "the reason is not said");
    }

    /// The box cannot draw a character width, a baseline shift or a line of mixed sizes
    /// exactly as the page will; each control says so when it is pointed at.
    #[test]
    fn what_the_box_draws_only_nearly_is_said_on_hover() {
        use egui_kittest::kittest::Queryable;
        let (mut h, _) = panel(look(), Gates::default());
        for (control, said) in [("Character width", "does not squeeze"), ("Baseline shift", "the page moves them"), ("size", "a little low")] {
            match control {
                // The size is the panel's first number.
                "size" => h.query_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the size field").hover(),
                name => h.get_by_label(name).hover(),
            }
            h.run_steps(4);
            assert!(h.query_by_label_contains(said).is_some(), "pointing at {control} does not say \"{said}\"");
        }
    }

    #[test]
    fn nothing_is_a_change_until_something_is_clicked() {
        assert!(!Changes::default().any());
        assert!(Changes { bold: Some(true), ..Default::default() }.any());
    }

    #[test]
    fn the_size_menu_offers_ascending_sizes_and_the_usual_ones() {
        assert!(SIZES.windows(2).all(|w| w[0] < w[1]));
        for usual in [8.0, 10.0, 12.0, 14.0, 24.0, 72.0] {
            assert!(SIZES.contains(&usual), "{usual}");
        }
    }

    #[test]
    fn the_italic_letter_is_drawn_from_the_font_atlas() {
        use egui_kittest::kittest::Queryable;
        let look = look();
        let mut h = egui_kittest::Harness::new_ui(|ui| {
            show(ui, egui::Id::new("style"), &look, &Gates::default());
        });
        h.run();
        let button = h.get_by_label("Italic").rect();
        let primitives = h.ctx.tessellate(h.output().shapes.clone(), h.ctx.pixels_per_point());
        let uvs: Vec<egui::Pos2> = primitives
            .iter()
            .filter_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => Some(m),
                _ => None,
            })
            .flat_map(|m| m.vertices.iter())
            .filter(|v| button.contains(v.pos))
            .map(|v| v.uv)
            .collect();
        assert!(!uvs.is_empty(), "nothing is drawn on the Italic button");
        assert!(
            uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)),
            "the letter samples outside the font atlas: {:?}",
            &uvs[..uvs.len().min(4)]
        );
    }

    #[test]
    fn a_switched_off_panel_has_a_reason_for_every_control() {
        let g = Gates::all_off("coming");
        for reason in [
            &g.bold,
            &g.italic,
            &g.underline,
            &g.strike,
            &g.script,
            &g.left,
            &g.center,
            &g.right,
            &g.justify,
            &g.indent,
            &g.lists,
            &g.letter_spacing,
            &g.width,
            &g.baseline,
            &g.line_spacing,
            &g.paragraph_spacing,
        ] {
            assert_eq!(reason.as_deref(), Some("coming"));
        }
        let _ = look();
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;

    const FONTS: [&str; 8] = [
        "Arial", "Arial Bold", "Arial Italic", "Arial Bold Italic", "Montserrat-Regular", "Montserrat-Bold", "Verdana", "Verdana Bold",
    ];

    #[test]
    fn a_name_says_what_style_it_is() {
        assert_eq!(style_of("Arial"), (false, false));
        assert_eq!(style_of("Arial Bold"), (true, false));
        assert_eq!(style_of("ABCDEF+Montserrat-BoldItalic"), (true, true));
        assert_eq!(style_of("Georgia Oblique"), (false, true));
    }

    #[test]
    fn the_family_is_the_name_without_its_style_and_weight() {
        assert_eq!(family_of("ABCDEF+Montserrat-Thin"), "montserrat");
        assert_eq!(family_of("Arial Bold Italic"), "arial");
        assert_eq!(family_of("Source Sans Pro Light"), "source sans pro");
    }

    #[test]
    fn bold_and_italic_come_from_the_same_family() {
        assert_eq!(family_variant("Arial", true, false, &FONTS).as_deref(), Some("Arial Bold"));
        assert_eq!(family_variant("Arial Bold", true, true, &FONTS).as_deref(), Some("Arial Bold Italic"));
        assert_eq!(family_variant("Arial Bold Italic", false, false, &FONTS).as_deref(), Some("Arial"));
        assert_eq!(family_variant("ABCDEF+Montserrat-Thin", true, false, &FONTS).as_deref(), Some("Montserrat-Bold"));
        assert_eq!(family_variant("Montserrat-Bold", false, false, &FONTS).as_deref(), Some("Montserrat-Regular"));
    }

    /// Windows lists Arial by its PostScript names, which carry Monotype's `MT`:
    /// `ArialMT`, `Arial-BoldMT`, `Arial-ItalicMT`, `Arial-BoldItalicMT`.
    #[test]
    fn monotype_names_find_their_bold_and_italic() {
        const MT: [&str; 6] =
            ["ArialMT", "Arial-BoldMT", "Arial-ItalicMT", "Arial-BoldItalicMT", "TimesNewRomanPSMT", "TimesNewRomanPS-BoldMT"];
        assert_eq!(family_variant("ArialMT", true, false, &MT).as_deref(), Some("Arial-BoldMT"));
        assert_eq!(family_variant("ArialMT", false, true, &MT).as_deref(), Some("Arial-ItalicMT"));
        assert_eq!(family_variant("ArialMT", true, true, &MT).as_deref(), Some("Arial-BoldItalicMT"));
        assert_eq!(family_variant("Arial-BoldMT", false, false, &MT).as_deref(), Some("ArialMT"));
        assert_eq!(family_variant("TimesNewRomanPSMT", true, false, &MT).as_deref(), Some("TimesNewRomanPS-BoldMT"));
        assert_eq!(style_of("Arial-BoldItalicMT"), (true, true));
    }

    #[test]
    fn a_family_without_the_face_gives_none_not_another_family() {
        assert_eq!(family_variant("Verdana", false, true, &FONTS), None);
        assert_eq!(family_variant("Montserrat-Regular", true, true, &FONTS), None);
        assert_eq!(family_variant("", true, false, &FONTS), None);
    }
}
