//! Commands for a page that carries its own `/Rotate`.
//!
//! The app speaks in the **displayed** frame (see [`crate::document::frame`]);
//! the engine's edits are verified in the unrotated one. [`Command::unturned`]
//! is the one place a command crosses from the first to the second: every point,
//! rectangle and offset in it is carried back, and the few quantities that depend
//! on which way a page is turned (a scale's two axes, a picture's rotation, the
//! direction a new line of text runs) are corrected for it. A command on a page
//! that is not turned comes out as it went in.

use super::Command;
use crate::document::frame::{turn_pixels, Frame};
use crate::document::{Annotation, Glyph, Point, TextLineEdit, TextStyle};

impl Command {
    /// This command, as the engine must run it, given the frame of each page.
    pub fn unturned(self, frame_of: &dyn Fn(usize) -> Frame) -> Command {
        match self {
            Command::SetPageCrop { index, crop } => {
                Command::SetPageCrop { index, crop: frame_of(index).rect_back(crop) }
            }
            Command::SetTextRun { page_index, object, text, mut style } => {
                unturn_style(&mut style, frame_of(page_index));
                Command::SetTextRun { page_index, object, text, style }
            }
            Command::SetTextRuns { page_index, mut edits } => {
                let f = frame_of(page_index);
                for (_, _, style) in &mut edits {
                    unturn_style(style, f);
                }
                Command::SetTextRuns { page_index, edits }
            }
            Command::ReplaceTextLines { page_index, mut edits } => {
                let f = frame_of(page_index);
                for edit in &mut edits {
                    if let TextLineEdit::Retype { style, .. } = edit {
                        unturn_style(style, f);
                    }
                }
                Command::ReplaceTextLines { page_index, edits }
            }
            Command::MoveObject { page_index, object, by } => {
                Command::MoveObject { page_index, object, by: offset_back(frame_of(page_index), by) }
            }
            Command::MoveObjects { page_index, objects, by } => {
                Command::MoveObjects { page_index, objects, by: offset_back(frame_of(page_index), by) }
            }
            Command::ScaleObjects { page_index, objects, anchor, sx, sy } => {
                let f = frame_of(page_index);
                let (sx, sy) = scales_back(f, sx, sy);
                Command::ScaleObjects { page_index, objects, anchor: f.point_back(anchor), sx, sy }
            }
            Command::ScaleObject { page_index, object, anchor, sx, sy } => {
                let f = frame_of(page_index);
                let (sx, sy) = scales_back(f, sx, sy);
                Command::ScaleObject { page_index, object, anchor: f.point_back(anchor), sx, sy }
            }
            Command::RotateObject { page_index, object, pivot, degrees } => {
                // A turn of the page is a rotation, not a reflection: turning an
                // object by some angle is the same angle in either frame.
                Command::RotateObject { page_index, object, pivot: frame_of(page_index).point_back(pivot), degrees }
            }
            Command::SetPlacedImageRect { page_index, index, rect, was } => {
                // A picture on a turned page is shown with its pixels turned with
                // the page (`Frame::image_mark`), so its own rotation, about its
                // centre, is the same in both frames and only the rectangle moves.
                let f = frame_of(page_index);
                Command::SetPlacedImageRect { page_index, index, rect: f.rect_back(rect), was: f.rect_back(was) }
            }
            Command::PasteLifted { page_index, lifted, at } => {
                Command::PasteLifted { page_index, lifted, at: frame_of(page_index).point_back(at) }
            }
            Command::AddAnnotation { page_index, annotation } => {
                let f = frame_of(page_index);
                Command::AddAnnotation { page_index, annotation: annotation.unturned(f) }
            }
            Command::AddTextLayer { page_index, mut words } => {
                let f = frame_of(page_index);
                for word in &mut words {
                    word.rect = f.rect_back(word.rect);
                }
                Command::AddTextLayer { page_index, words }
            }
            Command::Redact { page_index, area, fill, allow_incomplete, outlined_fonts } => Command::Redact {
                page_index,
                area: frame_of(page_index).rect_back(area),
                fill,
                allow_incomplete,
                outlined_fonts,
            },
            Command::Batch { commands } => {
                Command::Batch { commands: commands.into_iter().map(|c| c.unturned(frame_of)).collect() }
            }
            other => other,
        }
    }

    /// Whether running this command can change what [`Frame`] a page has: a
    /// page turned, cut, resized, moved, added or taken away.
    pub fn changes_frames(&self) -> bool {
        match self {
            Command::ReorderPages { .. }
            | Command::DeletePage { .. }
            | Command::InsertBlankPage { .. }
            | Command::SetPageRotation { .. }
            | Command::SetPageCrop { .. }
            | Command::SetPageSize { .. }
            | Command::ImportPages { .. }
            | Command::ReplacePage { .. } => true,
            Command::Batch { commands } => commands.iter().any(Command::changes_frames),
            _ => false,
        }
    }
}

fn unturn_style(style: &mut TextStyle, f: Frame) {
    if let Some((x, y)) = style.at {
        let p = f.point_back(Point { x, y });
        style.at = Some((p.x, p.y));
    }
}

fn offset_back(f: Frame, by: Point) -> Point {
    let (x, y) = f.vector_back((by.x, by.y));
    Point { x, y }
}

/// A scale along the displayed x and y, as a scale along the unrotated ones:
/// a quarter turn exchanges the two axes.
fn scales_back(f: Frame, sx: f32, sy: f32) -> (f32, f32) {
    if f.swaps_axes() {
        (sy, sx)
    } else {
        (sx, sy)
    }
}

fn carry_glyphs(glyphs: &mut [Glyph], f: Frame, back: bool) {
    for g in glyphs {
        let p = carry_point(f, Point { x: g.x, y: g.y }, back);
        g.x = p.x;
        g.y = p.y;
        // Angles in the app turn clockwise, as the page does: a glyph that
        // reads level on the turned page is rotated the other way on the sheet.
        g.radians += if back { -f.quarter_radians() } else { f.quarter_radians() };
    }
}

fn carry_point(f: Frame, p: Point, back: bool) -> Point {
    if back {
        f.point_back(p)
    } else {
        f.point(p)
    }
}

fn carry_rect(f: Frame, r: crate::document::Rect, back: bool) -> crate::document::Rect {
    if back {
        f.rect_back(r)
    } else {
        f.rect(r)
    }
}

impl Annotation {
    /// This annotation as the page shows it (from the engine's frame to the displayed one).
    pub fn turned(self, f: Frame) -> Annotation {
        carry_annotation(self, f, false)
    }

    /// This annotation as the engine must write it (from the displayed frame back).
    pub fn unturned(self, f: Frame) -> Annotation {
        carry_annotation(self, f, true)
    }
}

fn carry_annotation(annotation: Annotation, f: Frame, back: bool) -> Annotation {
    if f.is_identity() {
        return annotation;
    }
    let points = |ps: Vec<Point>| ps.into_iter().map(|p| carry_point(f, p, back)).collect::<Vec<_>>();
    match annotation {
        Annotation::Highlight { rects, color } => {
            Annotation::Highlight { rects: rects.into_iter().map(|r| carry_rect(f, r, back)).collect(), color }
        }
        Annotation::Underline { rects, color } => {
            Annotation::Underline { rects: rects.into_iter().map(|r| carry_rect(f, r, back)).collect(), color }
        }
        Annotation::StrikeOut { rects, color } => {
            Annotation::StrikeOut { rects: rects.into_iter().map(|r| carry_rect(f, r, back)).collect(), color }
        }
        Annotation::Squiggly { rects, color } => {
            Annotation::Squiggly { rects: rects.into_iter().map(|r| carry_rect(f, r, back)).collect(), color }
        }
        Annotation::Ink { strokes, color, width } => {
            Annotation::Ink { strokes: strokes.into_iter().map(points).collect(), color, width }
        }
        Annotation::Note { rect, contents, color } => Annotation::Note { rect: carry_rect(f, rect, back), contents, color },
        Annotation::Link { rect, uri } => Annotation::Link { rect: carry_rect(f, rect, back), uri },
        Annotation::Fill { outline, fill_color, stroke_color, width } => {
            Annotation::Fill { outline: points(outline), fill_color, stroke_color, width }
        }
        Annotation::Image { rect, rgba, width, height } => {
            // The pixels are turned the other way so that, once the page turns,
            // the picture reads upright and the right way round.
            let quarters = if back { (4 - f.turns()) % 4 } else { f.turns() };
            let (rgba, width, height) = turn_pixels(&rgba, width, height, quarters);
            Annotation::Image { rect: carry_rect(f, rect, back), rgba, width, height }
        }
        Annotation::Text {
            text,
            font,
            font_asset,
            size,
            color,
            mut glyphs,
            id,
            restore,
            frame,
            frame_width,
            scale_x,
            mut more,
            mut rules,
        } => {
            carry_glyphs(&mut glyphs, f, back);
            for part in &mut more {
                carry_glyphs(&mut part.glyphs, f, back);
            }
            for rule in &mut rules {
                rule.from = carry_point(f, rule.from, back);
                rule.to = carry_point(f, rule.to, back);
            }
            Annotation::Text {
                text,
                font,
                font_asset,
                size,
                color,
                glyphs,
                id,
                restore,
                frame: points(frame),
                frame_width,
                scale_x,
                more,
                rules,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Rect;

    fn turned() -> Frame {
        Frame::new(3, 2384.0, 1684.0)
    }

    fn of(_: usize) -> Frame {
        turned()
    }

    #[test]
    fn a_page_that_is_not_turned_runs_the_command_it_was_given() {
        let c = Command::MoveObjects { page_index: 0, objects: vec![1, 2], by: Point { x: 3.0, y: -4.0 } };
        let same = c.clone().unturned(&|_| Frame::NONE);
        assert_eq!(format!("{c:?}"), format!("{same:?}"));
    }

    #[test]
    fn a_move_to_the_right_on_screen_is_a_move_down_the_sheet() {
        // /Rotate 270: the sheet's +y is the screen's +x.
        let c = Command::MoveObject { page_index: 0, object: 4, by: Point { x: 10.0, y: 0.0 } }.unturned(&of);
        let Command::MoveObject { by, .. } = c else { panic!("{c:?}") };
        assert!((by.x - 0.0).abs() < 1e-4 && (by.y - 10.0).abs() < 1e-4, "{by:?}");
    }

    #[test]
    fn a_redact_area_lands_on_the_text_it_was_drawn_over() {
        let on_screen = Rect { left: 1648.0, top: 1004.0, right: 1663.0, bottom: 1012.0 };
        let c = Command::Redact {
            page_index: 0,
            area: on_screen,
            fill: None,
            allow_incomplete: false,
            outlined_fonts: vec![],
        }
        .unturned(&of);
        let Command::Redact { area, .. } = c else { panic!("{c:?}") };
        // The "142" label's rect on the sheet itself.
        assert!((area.left - 672.0).abs() < 1e-3 && (area.right - 680.0).abs() < 1e-3, "{area:?}");
        assert!((area.top - 1648.0).abs() < 1e-3 && (area.bottom - 1663.0).abs() < 1e-3, "{area:?}");
    }

    #[test]
    fn a_scale_swaps_its_axes_on_a_page_turned_a_quarter() {
        let c = Command::ScaleObject {
            page_index: 0,
            object: 1,
            anchor: Point { x: 0.0, y: 0.0 },
            sx: 2.0,
            sy: 1.0,
        }
        .unturned(&of);
        let Command::ScaleObject { sx, sy, .. } = c else { panic!("{c:?}") };
        assert_eq!((sx, sy), (1.0, 2.0));
    }

    #[test]
    fn a_batch_converts_every_member_and_knows_when_a_frame_may_change() {
        let c = Command::Batch {
            commands: vec![
                Command::MoveObject { page_index: 0, object: 1, by: Point { x: 1.0, y: 0.0 } },
                Command::SetPageRotation { index: 0, quarter_turns: 1 },
            ],
        };
        assert!(c.changes_frames());
        assert!(!Command::RemoveObject { page_index: 0, object: 1 }.changes_frames());
        let Command::Batch { commands } = c.unturned(&of) else { panic!() };
        let Command::MoveObject { by, .. } = &commands[0] else { panic!() };
        assert!((by.y - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_pasted_picture_is_turned_so_that_it_reads_upright_after_the_page_turns() {
        // 2 x 1 pixels: red then blue. Page turned 270 clockwise, so the sheet
        // holds the picture turned 90 clockwise: a column, red above blue.
        let rgba = vec![255, 0, 0, 255, 0, 0, 255, 255];
        let c = Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Image {
                rect: Rect { left: 100.0, top: 100.0, right: 120.0, bottom: 110.0 },
                rgba,
                width: 2,
                height: 1,
            },
        }
        .unturned(&of);
        let Command::AddAnnotation { annotation: Annotation::Image { rgba, width, height, rect }, .. } = c else {
            panic!("{c:?}")
        };
        assert_eq!((width, height), (1, 2));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255], "red on top");
        assert_eq!(&rgba[4..8], &[0, 0, 255, 255], "blue below");
        assert!((rect.right - rect.left - 10.0).abs() < 1e-3 && (rect.bottom - rect.top - 20.0).abs() < 1e-3);
    }

    #[test]
    fn new_text_is_written_turned_the_other_way() {
        let g = Glyph { ch: "A".into(), id: 0, x: 1650.0, y: 1000.0, radians: 0.0 };
        let c = Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Text {
                text: "A".into(),
                font: "Helvetica".into(),
                font_asset: None,
                size: 12.0,
                color: crate::document::Color { r: 0, g: 0, b: 0, a: 255 },
                glyphs: vec![g],
                id: 1,
                restore: String::new(),
                frame: vec![],
                frame_width: 0.0,
                scale_x: 1.0,
                more: vec![],
                rules: vec![],
            },
        }
        .unturned(&of);
        let Command::AddAnnotation { annotation: Annotation::Text { glyphs, .. }, .. } = c else { panic!() };
        // On the sheet the line runs the way the page's x axis points after three
        // clockwise quarter turns are undone: -270 degrees.
        assert!((glyphs[0].radians + 3.0 * std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{}", glyphs[0].radians);
        assert!((glyphs[0].x - (1684.0 - 1000.0)).abs() < 1e-3 && (glyphs[0].y - 1650.0).abs() < 1e-3);
    }
}
