//! The start screen — the Tool Wizard and Recent Documents from the mockup.
//!
//! Shown when nothing is open, which is the first thing anyone sees. Its whole
//! job is to get a document open in one click, which is the thing the app could
//! not do at all until the file dialog landed.
//!
//! Every card and every row emits a command string rather than calling a
//! function, for the same reason the ribbon does: §7's claim is that anything
//! clickable is typeable, and a control that reached past the command box would
//! be a fourth thing the Automate tab could not record.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
use pagify_shell::outlined_fonts::OutlinedFonts;
use pagify_shell::recent::{format_time, Recent};

use crate::theme;

/// One Tool Wizard card.
/// Which icon a card draws.
///
/// Drawn from primitives rather than set as a glyph. The obvious characters —
/// ✎, ⇵, ⇥ — are not in egui's bundled font and come out as empty boxes, and
/// bundling a font for three icons is a megabyte for nothing.
#[derive(Clone, Copy)]
enum Icon {
    Pencil,
    Merge,
    Extract,
}

struct Card {
    icon: Icon,
    title: &'static str,
    blurb: &'static str,
    command: &'static str,
    top: Color32,
    bottom: Color32,
}

/// The mockup's three cards.
///
/// Its third is labelled "Convert", which Pagify cannot do — there is no
/// conversion engine and none is planned in the build plan. A card that does
/// nothing is worse than one that does something, so the slot holds Extract,
/// which is real and is the operation people reach for next to Merge.
const CARDS: [Card; 3] = [
    Card {
        icon: Icon::Pencil,
        title: "Edit",
        blurb: "Open a PDF and mark it up — lines, arcs and dimensions, with object snap.",
        command: "open",
        top: theme::VIOLET_BRIGHT,
        bottom: theme::VIOLET_DEEP,
    },
    Card {
        icon: Icon::Merge,
        title: "Merge",
        blurb: "Bring pages in from another document and put them where you want them.",
        command: "import",
        top: theme::VIOLET,
        bottom: Color32::from_rgb(0x4C, 0x35, 0xA8),
    },
    Card {
        icon: Icon::Extract,
        title: "Extract",
        blurb: "Pull a range of pages out into a new document of their own.",
        command: "extract",
        top: theme::BLUE,
        bottom: Color32::from_rgb(0x2B, 0x5A, 0xB8),
    },
];

/// Draw the start screen. Returns a command string if something was clicked.
pub fn show(ui: &mut Ui, recent: &Recent, outlined_fonts: &OutlinedFonts) -> Option<String> {
    let mut command = None;

    ui.add_space(10.0);
    heading(ui, "Tool Wizard");
    ui.add_space(6.0);

    // Three across when there is room, stacking when there is not — the panel
    // is resizable and three fixed cards would clip.
    let available = ui.available_width();
    let card_width = ((available - 24.0) / 3.0).max(180.0);

    ui.horizontal_wrapped(|ui| {
        for card in &CARDS {
            if draw_card(ui, card, card_width) {
                command = Some(card.command.to_string());
            }
        }
    });

    ui.add_space(16.0);

    // Fixed budget for the fonts panel below, so it and Recent Documents
    // divide the remaining space instead of the second one being squeezed to
    // nothing on a short window.
    const FONTS_PANEL_HEIGHT: f32 = 128.0;
    let remaining = ui.available_height();

    // Recent documents, in a bordered panel like the mockup's.
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            // The mockup's recents panel runs to the bottom of the window, so
            // it claims what is left rather than shrinking to its rows.
            ui.set_min_height((remaining - FONTS_PANEL_HEIGHT - 40.0).max(120.0));
            ui.set_min_width(ui.available_width());
            heading(ui, "Recent Documents");
            ui.add_space(8.0);

            let entries = recent.present();
            if entries.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.colored_label(theme::INK_FAINT, "Nothing yet — open a PDF and it will appear here.");
                });
                ui.add_space(12.0);
                return;
            }

            if let Some(path) = table(ui, &entries) {
                // Quoted, so a path with spaces survives the round trip through
                // the command box exactly as a typed one does.
                command = Some(format!("open \"{}\"", path));
            }
        });

    ui.add_space(12.0);

    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_height(FONTS_PANEL_HEIGHT);
            ui.set_min_width(ui.available_width());
            if let Some(clicked) = fonts_section(ui, outlined_fonts) {
                command = Some(clicked);
            }
        });

    command
}

/// A page whose words are drawn as glyph outlines rather than real text reads
/// perfectly and cannot be searched or selected — see `PageTextKind::Outlined`
/// in `pdf_core`. Vector matching fixes that without OCR, but only for a font
/// the catalogue actually has glyphs from. This is where a reader supplies
/// one of their own, for the document in front of them that the bundled
/// Montserrat does not cover.
fn fonts_section(ui: &mut Ui, fonts: &OutlinedFonts) -> Option<String> {
    let mut command = None;

    heading(ui, "Outlined-Text Fonts");
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(
            "Tried, alongside the bundled Montserrat, on a page whose text turns out to be drawn as outlines.",
        )
        .color(theme::INK_DIM)
        .font(FontId::proportional(12.0)),
    );
    ui.add_space(8.0);

    let present = fonts.present();
    if present.is_empty() {
        ui.colored_label(theme::INK_FAINT, "No extra fonts added.");
    } else {
        for path in &present {
            ui.horizontal(|ui| {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(egui::RichText::new(name).color(theme::INK).font(FontId::proportional(13.0)));
                if ui.small_button("Remove").clicked() {
                    command = Some(format!("outlinedfont remove \"{}\"", path.display()));
                }
            });
        }
    }

    ui.add_space(8.0);
    if ui.button("Add font…").clicked() {
        command = Some("outlinedfont".to_string());
    }

    command
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(theme::INK)
            .font(FontId::proportional(16.0)),
    );
}

fn draw_card(ui: &mut Ui, card: &Card, width: f32) -> bool {
    let height = 92.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    let hovered = response.hovered();
    let painter = ui.painter();

    painter.rect_filled(rect, CornerRadius::same(10), theme::PANEL);
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, if hovered { theme::VIOLET } else { theme::LINE }),
        StrokeKind::Inside,
    );

    // Icon tile.
    let tile = Rect::from_min_size(rect.min + Vec2::new(14.0, 16.0), Vec2::splat(44.0));
    theme::icon_tile(painter, tile, card.top, card.bottom);
    draw_icon(painter, tile, card.icon);

    let text_left = tile.right() + 14.0;
    painter.text(
        egui::pos2(text_left, rect.min.y + 20.0),
        Align2::LEFT_TOP,
        card.title,
        FontId::proportional(16.0),
        theme::INK,
    );

    // Wrapped by hand: `Painter::text` does not wrap, and the blurbs are two
    // lines in the mockup.
    let wrap_at = rect.right() - text_left - 12.0;
    let galley = painter.layout(
        card.blurb.to_string(),
        FontId::proportional(12.0),
        theme::INK_DIM,
        wrap_at,
    );
    painter.galley(egui::pos2(text_left, rect.min.y + 44.0), galley, theme::INK_DIM);

    response.clicked()
}

/// The Recent Documents table. Returns the path of a clicked row.
fn table(ui: &mut Ui, entries: &[&pagify_shell::recent::Entry]) -> Option<String> {
    let mut clicked = None;
    let row_height = 30.0;

    TableBuilder::new(ui)
        .striped(true)
        .sense(Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::remainder().at_least(180.0))
        .column(Column::remainder().at_least(180.0))
        .column(Column::exact(150.0))
        .header(28.0, |mut header| {
            for title in ["Name", "Location", "Date Modified"] {
                header.col(|ui| {
                    // A lighter band behind the whole row, as in the mockup.
                    // A table header hands out one rect per cell, so the bands
                    // are painted wide enough to close the gap at each column
                    // boundary — three separate strips with seams between them
                    // is what a tight rect looks like.
                    ui.painter().rect_filled(
                        ui.max_rect().expand2(egui::vec2(20.0, 5.0)),
                        CornerRadius::ZERO,
                        theme::HEADER,
                    );
                    ui.label(
                        egui::RichText::new(title)
                            .color(theme::INK_DIM)
                            .font(FontId::proportional(12.5)),
                    );
                });
            }
        })
        .body(|body| {
            body.rows(row_height, entries.len(), |mut row| {
                let entry = entries[row.index()];

                row.col(|ui| {
                    pdf_icon(ui);
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(entry.name()).color(theme::INK).font(FontId::proportional(13.0)));
                });
                row.col(|ui| {
                    // Elided at the front, so the filename end of a long path
                    // stays readable — which is the part that identifies it.
                    ui.label(
                        egui::RichText::new(elide(&entry.location(), 46))
                            .color(theme::INK_DIM)
                            .font(FontId::proportional(12.5)),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(format_time(entry.opened_at))
                            .color(theme::INK_DIM)
                            .font(FontId::proportional(12.5)),
                    );
                });

                if row.response().clicked() {
                    clicked = Some(entry.path.to_string_lossy().into_owned());
                }
            });
        });

    clicked
}

/// The card icons, in white on the tile.
fn draw_icon(painter: &egui::Painter, tile: Rect, icon: Icon) {
    let c = tile.center();
    let r = tile.width() * 0.24;
    let pen = Stroke::new(1.8, Color32::WHITE);

    match icon {
        // A sheet with a stroke across it.
        Icon::Pencil => {
            let sheet = Rect::from_center_size(c, Vec2::new(r * 1.5, r * 1.9));
            painter.rect_stroke(sheet, CornerRadius::same(2), pen, StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(sheet.left() + 2.0, sheet.bottom() - 3.0),
                    egui::pos2(sheet.right() - 2.0, sheet.top() + 3.0),
                ],
                pen,
            );
        }
        // Two arrows converging: pages coming together.
        Icon::Merge => {
            for (from, to) in [
                (egui::pos2(c.x - r, c.y - r), egui::pos2(c.x, c.y)),
                (egui::pos2(c.x + r, c.y - r), egui::pos2(c.x, c.y)),
            ] {
                painter.line_segment([from, to], pen);
            }
            painter.line_segment([egui::pos2(c.x, c.y), egui::pos2(c.x, c.y + r)], pen);
            // Arrow head.
            painter.line_segment(
                [egui::pos2(c.x - 4.0, c.y + r - 4.0), egui::pos2(c.x, c.y + r)],
                pen,
            );
            painter.line_segment(
                [egui::pos2(c.x + 4.0, c.y + r - 4.0), egui::pos2(c.x, c.y + r)],
                pen,
            );
        }
        // A sheet with an arrow leaving it.
        Icon::Extract => {
            let sheet = Rect::from_min_size(
                egui::pos2(c.x - r * 1.4, c.y - r),
                Vec2::new(r * 1.3, r * 2.0),
            );
            painter.rect_stroke(sheet, CornerRadius::same(2), pen, StrokeKind::Inside);
            let tail = egui::pos2(sheet.right() + 3.0, c.y);
            let tip = egui::pos2(c.x + r * 1.4, c.y);
            painter.line_segment([tail, tip], pen);
            painter.line_segment([egui::pos2(tip.x - 5.0, tip.y - 4.0), tip], pen);
            painter.line_segment([egui::pos2(tip.x - 5.0, tip.y + 4.0), tip], pen);
        }
    }
}

fn elide(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    format!("…{}", chars[chars.len() - max + 1..].iter().collect::<String>())
}

/// A small red document glyph, drawn rather than shipped as an asset.
fn pdf_icon(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(13.0, 16.0), Sense::hover());
    let painter = ui.painter();
    let fold = 4.5;

    painter.rect_filled(rect, CornerRadius::same(2), theme::PDF_RED);
    // The turned-down corner, in the surface colour behind it.
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(rect.right() - fold, rect.top()),
            egui::pos2(rect.right(), rect.top()),
            egui::pos2(rect.right(), rect.top() + fold),
        ],
        theme::PANEL,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wizard_card_runs_a_command_the_box_understands() {
        // The same guard the ribbon has: a card whose command string is a typo
        // is a card that does nothing, and nothing else would catch it.
        for card in &CARDS {
            match pagify_shell::command::dispatch(card.command) {
                Some(pagify_shell::command::Dispatch::Unknown(token)) => {
                    panic!("card `{}` runs `{}`, and `{token}` is not a command", card.title, card.command)
                }
                Some(pagify_shell::command::Dispatch::Refused { token, .. }) => {
                    panic!("card `{}` runs a refused command ({token})", card.title)
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_long_location_keeps_its_tail_because_that_is_the_identifying_part() {
        let long = "/Users/someone/Documents/Projects/2026/Catalogues/Autumn";
        let shown = elide(long, 20);
        assert!(shown.starts_with('…'));
        assert!(shown.ends_with("Autumn"), "elided the wrong end: {shown}");
        assert_eq!(elide("/tmp", 20), "/tmp", "short paths are left alone");
    }
}
