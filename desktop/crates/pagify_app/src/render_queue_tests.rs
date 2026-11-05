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
fn a_render_gives_up_for_a_newer_ask_of_its_picture_or_a_more_urgent_one() {
    let s = session();
    let thumb = THUMB_STEP_BASE + 160;
    let page = job(&s, 2, EXACT_STEP_BASE, 0);
    assert!(!overtaken(&page, &[]), "nothing waiting");
    assert!(!overtaken(&page, &[job(&s, 3, EXACT_STEP_BASE, 0), job(&s, 2, thumb, 0)]), "another page, or this page's thumbnail, is no reason");
    assert!(overtaken(&page, &[job(&s, 2, EXACT_STEP_BASE + 1, 0)]), "a newer size of the same page makes this one no use");
    let textless = RenderJob { hide: Some(std::sync::Arc::from([1usize])), ..job(&s, 3, EXACT_STEP_BASE, 0) };
    assert!(overtaken(&page, &[textless]), "the editor's textless picture goes first");

    let thumbnail = job(&s, 5, thumb, 0);
    assert!(overtaken(&thumbnail, &[job(&s, 1, EXACT_STEP_BASE, 0)]), "a page asked for while a thumbnail draws");
    assert!(!overtaken(&thumbnail, &[job(&s, 4, thumb, 0)]), "another thumbnail waits its turn");
}

/// Frames until every page has a thumbnail, or give up.
fn until_every_page_has_a_thumbnail(h: &mut egui_kittest::Harness<'static, PagifyApp>) -> bool {
    for _ in 0..1000 {
        let doc = h.state().tab().doc.as_ref().expect("doc");
        if (0..doc.page_count).all(|p| doc.caches.thumbs.contains_key(&p)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    false
}

/// **The rail's thumbnails are drawn by hayro, beside PDFium** (the owner's
/// choice, 2026-11-05): with pages rendered off the UI thread, they come back
/// from the preview threads.
#[test]
fn the_rails_thumbnails_are_drawn_by_hayro() {
    let mut h = crate::ui_tests::async_harness(&crate::ui_tests::fixture("pages-ladder.pdf"));
    assert!(until_every_page_has_a_thumbnail(&mut h), "the rail never filled");
    let thumbs = h.state().tab().doc.as_ref().expect("doc").caches.thumbs.len() as u64;
    let previewed = h.state().render_state.render_stats.previewed;
    assert!(previewed >= thumbs && thumbs > 0, "{previewed} thumbnails came from hayro of the {thumbs} in the rail");
}

/// A page hayro cannot draw still gets its thumbnail — from PDFium, as before.
#[test]
fn a_thumbnail_hayro_cannot_draw_is_drawn_by_pdfium() {
    let mut h = crate::ui_tests::async_harness(&crate::ui_tests::fixture("pages-ladder.pdf"));
    let ctx = h.ctx.clone();
    let (doc, epoch) = {
        let d = h.state().tab().doc.as_ref().expect("doc");
        (d.id, d.render_epoch)
    };
    // hayro has already said no to the first page, in this state of the document.
    h.state_mut().tab_mut().doc.as_mut().expect("doc").caches.thumbs.clear();
    let mut worker = crate::previews::PreviewWorker::start(&ctx).expect("threads");
    worker.refused.insert((doc, epoch, 0));
    h.state_mut().render_state.previews = Some(worker);
    h.state_mut().render_state.render_stats.requested = 0;
    assert!(until_every_page_has_a_thumbnail(&mut h), "the page hayro refused got no thumbnail");
    // hayro draws the rest, page pictures too; only the refused page is PDFium's.
    assert!(h.state().render_state.render_stats.requested >= 1, "PDFium was never asked for the refused page");
}

/// Frames until `done` says so, or give up.
fn until(h: &mut egui_kittest::Harness<'static, PagifyApp>, done: impl Fn(&PagifyApp) -> bool) -> bool {
    for _ in 0..1000 {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    false
}

fn page_0_pictured(app: &PagifyApp) -> bool {
    app.tab().doc.as_ref().is_some_and(|d| d.caches.exact.keys().any(|(p, _, _)| *p == 0))
}

/// **Page pictures are drawn by hayro, where it draws them as PDFium does**
/// (the owner's decision, 2026-11-05, after the comparison of §51): the page
/// in view, and the rail, without PDFium.
#[test]
fn a_page_is_drawn_by_hayro() {
    let mut h = crate::ui_tests::async_harness(&crate::ui_tests::fixture("pages-ladder.pdf"));
    assert!(until(&mut h, page_0_pictured), "page 1 never got its picture");
    let stats = h.state().render_state.render_stats;
    assert!(stats.previewed >= 1, "{stats:?}");
    assert_eq!(stats.requested, 0, "PDFium was asked: {stats:?}");
}

/// A page hayro may draw differently — here, Pagify's own highlight, which has
/// no appearance stream — is shown in hayro's picture meanwhile and ends in
/// PDFium's, under the same key.
#[test]
fn a_page_hayro_may_draw_differently_ends_with_pdfiums_picture() {
    let session = Session::open(crate::ui_tests::fixture("two-column.pdf")).expect("open");
    let mark = pdf_core::document::Rect { left: 72.0, top: 72.0, right: 200.0, bottom: 90.0 };
    session.highlight(0, vec![mark], pdf_core::document::Color { r: 255, g: 230, b: 0, a: 255 }).expect("highlight");
    let path = std::env::temp_dir().join(format!("pagify-highlighted-{}.pdf", std::process::id()));
    session.save_to(&path, false).expect("save");
    let mut h = crate::ui_tests::async_harness(path.to_str().expect("path"));
    let pdfiums = |app: &PagifyApp| page_0_pictured(app) && app.render_state.render_stats.applied >= 1;
    assert!(until(&mut h, pdfiums), "PDFium's picture never arrived: {:?}", h.state().render_state.render_stats);
    let stats = h.state().render_state.render_stats;
    assert!(stats.requested >= 1, "{stats:?}");
    let _ = std::fs::remove_file(&path);
}

/// With "Exact" on, every picture is PDFium's.
#[test]
fn with_exact_pages_on_nothing_is_drawn_by_hayro() {
    // On before the first frame, which the harness draws as it is built.
    let mut h = crate::ui_tests::pdfium_harness(&crate::ui_tests::fixture("pages-ladder.pdf"));
    assert!(until(&mut h, page_0_pictured), "page 1 never got its picture");
    assert!(until_every_page_has_a_thumbnail(&mut h), "the rail never filled");
    let stats = h.state().render_state.render_stats;
    assert_eq!(stats.previewed, 0, "{stats:?}");
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
