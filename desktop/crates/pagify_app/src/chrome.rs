//! Pagify's own window chrome on Windows, where the system title bar is off
//! (`with_decorations(false)`) and Pagify's top row is the only title bar: the
//! minimise, maximise and close buttons, and the edge that resizes the window.

/// How far in from the window's edge a press resizes it.
pub(crate) const BAND: f32 = 5.0;

/// Windows' own caption buttons are this wide, and as tall as the bar.
const BUTTON_WIDTH: f32 = 46.0;

/// Which way a press at `at` resizes `window`: the edge it is within `band`
/// of, the corner when it is within `band` of two, `None` further in.
pub(crate) fn resize_direction(window: egui::Rect, at: egui::Pos2, band: f32) -> Option<egui::ResizeDirection> {
    use egui::ResizeDirection::*;
    let north = at.y < window.top() + band;
    let south = at.y > window.bottom() - band;
    let west = at.x < window.left() + band;
    let east = at.x > window.right() - band;
    Some(match (north, south, west, east) {
        (true, _, true, _) => NorthWest,
        (true, _, _, true) => NorthEast,
        (_, true, true, _) => SouthWest,
        (_, true, _, true) => SouthEast,
        (true, ..) => North,
        (_, true, ..) => South,
        (.., true, _) => West,
        (.., true) => East,
        _ => return None,
    })
}

/// The cursor that says which way the window will resize.
pub(crate) fn resize_cursor(direction: egui::ResizeDirection) -> egui::CursorIcon {
    use egui::{CursorIcon, ResizeDirection::*};
    match direction {
        North => CursorIcon::ResizeNorth,
        South => CursorIcon::ResizeSouth,
        West => CursorIcon::ResizeWest,
        East => CursorIcon::ResizeEast,
        NorthWest => CursorIcon::ResizeNorthWest,
        NorthEast => CursorIcon::ResizeNorthEast,
        SouthWest => CursorIcon::ResizeSouthWest,
        SouthEast => CursorIcon::ResizeSouthEast,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Caption {
    Close,
    Maximise,
    Restore,
    Minimise,
}

/// Minimise, maximise (restore while maximised) and close, drawn right to left
/// in a right-to-left row, as Windows draws them: thin lines, no icon font,
/// Close red under the pointer. Returns what the one clicked asks the system.
pub(crate) fn window_buttons(ui: &mut egui::Ui, maximized: bool) -> Option<egui::ViewportCommand> {
    let height = ui.available_height();
    let size = if maximized { Caption::Restore } else { Caption::Maximise };
    let mut command = None;
    for caption in [Caption::Close, size, Caption::Minimise] {
        let label = match caption {
            Caption::Close => "Close window",
            Caption::Maximise => "Maximise",
            Caption::Restore => "Restore",
            Caption::Minimise => "Minimise",
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(BUTTON_WIDTH, height), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let close = caption == Caption::Close;
            if response.hovered() {
                let fill = if close { egui::Color32::from_rgb(0xC4, 0x2B, 0x1C) } else { ui.visuals().widgets.hovered.weak_bg_fill };
                painter.rect_filled(rect, 0.0, fill);
            }
            let ink = if close && response.hovered() { egui::Color32::WHITE } else { crate::theme::ink() };
            let stroke = egui::Stroke::new(1.0, ink);
            let c = rect.center();
            let r = 5.0;
            match caption {
                Caption::Close => {
                    painter.line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], stroke);
                    painter.line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], stroke);
                }
                Caption::Maximise => {
                    painter.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r)), 0.0, stroke, egui::StrokeKind::Inside);
                }
                Caption::Restore => {
                    // Two windows, the back one showing above and to the right.
                    let front = egui::Rect::from_min_size(c + egui::vec2(-r, -r + 2.0), egui::vec2(2.0 * r - 2.0, 2.0 * r - 2.0));
                    painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
                    painter.line_segment([c + egui::vec2(-r + 2.0, -r), c + egui::vec2(r, -r)], stroke);
                    painter.line_segment([c + egui::vec2(r, -r), c + egui::vec2(r, r - 2.0)], stroke);
                }
                Caption::Minimise => {
                    painter.line_segment([c + egui::vec2(-r, 0.0), c + egui::vec2(r, 0.0)], stroke);
                }
            }
        }
        if response.on_hover_text(label).clicked() {
            command = Some(match caption {
                Caption::Close => egui::ViewportCommand::Close,
                Caption::Maximise => egui::ViewportCommand::Maximized(true),
                Caption::Restore => egui::ViewportCommand::Maximized(false),
                Caption::Minimise => egui::ViewportCommand::Minimized(true),
            });
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::ResizeDirection::*;

    fn window() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0))
    }

    fn at(x: f32, y: f32) -> Option<egui::ResizeDirection> {
        resize_direction(window(), egui::pos2(x, y), BAND)
    }

    #[test]
    fn each_edge_resizes_its_own_way() {
        assert_eq!(at(500.0, 2.0), Some(North));
        assert_eq!(at(500.0, 798.0), Some(South));
        assert_eq!(at(2.0, 400.0), Some(West));
        assert_eq!(at(998.0, 400.0), Some(East));
    }

    #[test]
    fn each_corner_resizes_both_ways() {
        assert_eq!(at(2.0, 2.0), Some(NorthWest));
        assert_eq!(at(998.0, 2.0), Some(NorthEast));
        assert_eq!(at(2.0, 798.0), Some(SouthWest));
        assert_eq!(at(998.0, 798.0), Some(SouthEast));
    }

    #[test]
    fn inside_the_band_is_not_a_resize() {
        assert_eq!(at(500.0, 400.0), None);
        assert_eq!(at(6.0, 400.0), None, "6 px in is past the 5 px band");
        assert_eq!(at(994.0, 400.0), None);
    }
}
