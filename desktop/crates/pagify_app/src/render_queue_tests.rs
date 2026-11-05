//! The render worker's queue, without its thread: what it draws next, and the
//! thumbnails it makes from a page's own picture (reported from use, 2026-11-05:
//! on a CAD set the page scrolled to waited behind 4.7 s of thumbnails).

use super::*;

fn job(session: &std::sync::Arc<Session>, page: usize, step: u32, epoch: u64) -> RenderJob {
    RenderJob { doc: 1, epoch, page, step, rotation: Rotation::None, scale: 1.0, crop: None, session: session.clone(), hide: None }
}

fn session() -> std::sync::Arc<Session> {
    std::sync::Arc::new(Session::open(crate::ui_tests::fixture("pictures.pdf")).expect("open"))
}

fn taken(pending: &mut Vec<RenderJob>) -> (usize, u32) {
    next_render(pending).map(|j| (j.page, j.step)).expect("a job")
}

#[test]
fn a_page_asked_after_thumbnails_is_drawn_before_them() {
    let s = session();
    let thumb = THUMB_STEP_BASE + 160;
    let mut pending = vec![job(&s, 5, thumb, 0), job(&s, 4, thumb, 0), job(&s, 0, EXACT_STEP_BASE, 0)];
    assert_eq!(taken(&mut pending), (0, EXACT_STEP_BASE));
    // Asked while page 1 drew: ahead of the thumbnails already waiting.
    pending.push(job(&s, 1, EXACT_STEP_BASE, 0));
    assert_eq!(taken(&mut pending), (1, EXACT_STEP_BASE));
    assert_eq!(taken(&mut pending), (4, thumb), "newest thumbnail first");
    assert_eq!(taken(&mut pending), (5, thumb));
    assert!(next_render(&mut pending).is_none());
}

#[test]
fn only_the_newest_ask_for_a_page_is_drawn() {
    let s = session();
    let mut pending = vec![job(&s, 0, 10, 0), job(&s, 0, 20, 0), job(&s, 0, THUMB_STEP_BASE + 160, 0)];
    assert_eq!(taken(&mut pending), (0, 20));
    assert_eq!(taken(&mut pending), (0, THUMB_STEP_BASE + 160), "the thumbnail is not the page's to replace");
    assert!(pending.is_empty());
}

#[test]
fn a_thumbnail_of_the_page_just_drawn_is_made_from_its_picture() {
    let s = session();
    let picture = egui::ColorImage::filled([400, 300], egui::Color32::from_rgb(10, 20, 30));
    let page = job(&s, 2, EXACT_STEP_BASE, 7);

    let mut pending = vec![job(&s, 2, THUMB_STEP_BASE + 100, 7)];
    let (thumb, image) = thumb_from(&mut pending, &page, &picture).expect("made from the picture");
    assert_eq!((thumb.page, thumb.step), (2, THUMB_STEP_BASE + 100));
    assert_eq!(image.size, [100, 75]);
    assert_eq!(image.pixels[0], egui::Color32::from_rgb(10, 20, 30));
    assert!(pending.is_empty(), "the thumbnail was still left to render");

    // Another state of the page, another page, or a turned picture: rendered as asked.
    for (mut pending, done) in [
        (vec![job(&s, 2, THUMB_STEP_BASE + 100, 8)], job(&s, 2, EXACT_STEP_BASE, 7)),
        (vec![job(&s, 3, THUMB_STEP_BASE + 100, 7)], job(&s, 2, EXACT_STEP_BASE, 7)),
        (vec![job(&s, 2, THUMB_STEP_BASE + 100, 7)], RenderJob { rotation: Rotation::Clockwise90, ..job(&s, 2, EXACT_STEP_BASE, 7) }),
        (vec![job(&s, 2, THUMB_STEP_BASE + 500, 7)], job(&s, 2, EXACT_STEP_BASE, 7)),
    ] {
        assert!(thumb_from(&mut pending, &done, &picture).is_none());
        assert_eq!(pending.len(), 1);
    }
}

#[test]
fn a_shrunk_picture_averages_what_each_pixel_covers() {
    let (black, white) = (egui::Color32::BLACK, egui::Color32::WHITE);
    // Left half black, right half white, 4 × 2 down to 2 × 1.
    let halves = egui::ColorImage::new([4, 2], vec![black, black, white, white, black, black, white, white]);
    assert_eq!(shrink(&halves, 2, 1).pixels, [black, white]);
    // A checkerboard down to one pixel is its grey.
    let board = egui::ColorImage::new([2, 2], vec![black, white, white, black]);
    assert_eq!(shrink(&board, 1, 1).pixels, [egui::Color32::from_rgba_premultiplied(127, 127, 127, 255)]);
}
