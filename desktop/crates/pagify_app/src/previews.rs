//! The rail's thumbnails, drawn by hayro on threads of their own
//! ([`pagify_shell::preview`]) instead of by the render worker's one PDFium
//! thread, where a thumbnail of a heavy page held the engine for what the page
//! itself cost while the page in view waited (reported from use, 2026-11-05: a
//! CAD set's rail took 4.7 s of PDFium; the owner's choice, thumbnails only).
//!
//! The threads share one hayro parse of each document state, read from the
//! session's bytes as it stands. A thumbnail is stamped with the document's
//! render epoch, like a render, and one for a page as it no longer is is
//! dropped. Where hayro cannot read a document or draw a page, that page's
//! thumbnail is asked of PDFium as before. Only with the render worker on
//! ([`crate::RenderState::async_render`]): the tests, which run without it,
//! render thumbnails where they ask for them, as before.

use pagify_shell::preview::Preview;
use pagify_shell::session::Session;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// A thumbnail to draw: page `page` of a document in the state `epoch`,
/// `width` pixels wide.
pub(crate) struct PreviewJob {
    pub(crate) doc: u64,
    pub(crate) epoch: u64,
    pub(crate) page: usize,
    pub(crate) width: u32,
    pub(crate) session: Arc<Session>,
}

/// What came back: the thumbnail, or `None` where hayro could not draw it.
pub(crate) struct PreviewDone {
    pub(crate) doc: u64,
    pub(crate) epoch: u64,
    pub(crate) page: usize,
    pub(crate) width: u32,
    pub(crate) image: Option<egui::ColorImage>,
}

/// Each document as hayro last read it: the state, and the parse — `None`
/// when hayro could not read that state.
type Read = Mutex<HashMap<u64, (u64, Option<Arc<Preview>>)>>;

/// The threads that draw thumbnails with hayro.
pub(crate) struct PreviewWorker {
    jobs: Sender<PreviewJob>,
    pub(crate) done: Receiver<PreviewDone>,
    /// `(document, page)` to the `(epoch, width)` asked for and not back yet.
    pub(crate) in_flight: HashMap<(u64, usize), (u64, u32)>,
    /// What hayro could not draw, as `(document, epoch, page)`: asked of PDFium.
    pub(crate) refused: HashSet<(u64, u64, usize)>,
}

impl PreviewWorker {
    pub(crate) fn start(ctx: &egui::Context) -> Option<Self> {
        let (jobs, queue) = channel::<PreviewJob>();
        let (reply, done) = channel::<PreviewDone>();
        let queue = Arc::new(Mutex::new(queue));
        let read: Arc<Read> = Arc::default();
        // As PdfCraft sizes its render pool: a core left for everything else.
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 8) - 1;
        for n in 0..threads {
            let (queue, reply, read, ctx) = (queue.clone(), reply.clone(), read.clone(), ctx.clone());
            std::thread::Builder::new()
                .name(format!("pagify-preview-{n}"))
                .spawn(move || loop {
                    let job = match queue.lock().map(|q| q.recv()) {
                        Ok(Ok(job)) => job,
                        _ => return,
                    };
                    // A newer state of the document has been read since this was
                    // asked: the page as it was is no use to anyone.
                    let Some(preview) = preview_of(&read, &job) else { continue };
                    let image = preview.and_then(|p| p.render(job.page, job.width)).map(|r| crate::page_to_image(&r));
                    let sent = reply.send(PreviewDone { doc: job.doc, epoch: job.epoch, page: job.page, width: job.width, image });
                    if sent.is_err() {
                        return;
                    }
                    ctx.request_repaint();
                })
                .ok()?;
        }
        Some(PreviewWorker { jobs, done, in_flight: HashMap::new(), refused: HashSet::new() })
    }
}

/// The parse a job is drawn from: read once a state, by whichever thread gets
/// there first, the others waiting for it. `Some(None)` when hayro cannot read
/// that state; `None` when the job is for a state older than the one read.
fn preview_of(read: &Read, job: &PreviewJob) -> Option<Option<Arc<Preview>>> {
    let mut read = read.lock().ok()?;
    match read.get(&job.doc) {
        Some((epoch, preview)) if *epoch == job.epoch => return Some(preview.clone()),
        Some((epoch, _)) if *epoch > job.epoch => return None,
        _ => {}
    }
    let preview = job.session.current_bytes().ok().and_then(Preview::new).map(Arc::new);
    // ponytail: a closed document's parse (its whole file) is only let go of
    // once four others have been read; tell the worker of closes if that grows.
    if read.len() >= 4 && !read.contains_key(&job.doc) {
        if let Some(other) = read.keys().next().copied() {
            read.remove(&other);
        }
    }
    read.insert(job.doc, (job.epoch, preview.clone()));
    Some(preview)
}

impl crate::PagifyApp {
    /// Ask hayro for page `page`'s thumbnail, `width` pixels wide. `false` when
    /// it cannot draw it — this page of this state, or there are no threads to
    /// draw on — for the caller to ask PDFium.
    pub(crate) fn ask_for_preview(&mut self, ctx: &egui::Context, page: usize, width: u32) -> bool {
        let Some(doc) = self.tab().doc.as_ref() else { return false };
        let (id, epoch, session) = (doc.id, doc.render_epoch, doc.session.clone());
        if self.render_state.previews.is_none() {
            self.render_state.previews = PreviewWorker::start(ctx);
        }
        let Some(worker) = self.render_state.previews.as_mut() else { return false };
        if worker.refused.contains(&(id, epoch, page)) {
            return false;
        }
        if worker.in_flight.get(&(id, page)) == Some(&(epoch, width)) {
            return true;
        }
        if worker.jobs.send(PreviewJob { doc: id, epoch, page, width, session }).is_err() {
            // The threads have gone: PDFium draws the thumbnails again.
            self.render_state.previews = None;
            return false;
        }
        worker.in_flight.insert((id, page), (epoch, width));
        true
    }

    /// The thumbnails hayro has drawn, filed — once a frame. One for a page as
    /// it no longer is, or for a document since closed, is dropped; one hayro
    /// could not draw sends that page to PDFium from then on.
    pub(crate) fn collect_previews(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.render_state.previews.as_mut() else { return };
        let mut arrived = Vec::new();
        while let Ok(done) = worker.done.try_recv() {
            if worker.in_flight.get(&(done.doc, done.page)) == Some(&(done.epoch, done.width)) {
                worker.in_flight.remove(&(done.doc, done.page));
            }
            if done.image.is_none() {
                worker.refused.insert((done.doc, done.epoch, done.page));
            }
            arrived.push(done);
        }
        for done in arrived {
            let Some(image) = done.image else { continue };
            let current = self
                .tabs
                .iter_mut()
                .filter_map(|t| t.doc.as_mut())
                .find(|d| d.id == done.doc && d.render_epoch == done.epoch);
            let Some(doc) = current else { continue };
            let handle = ctx.load_texture(
                format!("thumb{}", done.page),
                image,
                // As the render worker's thumbnails: mipmapped, so a bitmap a little
                // larger than it is shown is averaged rather than aliased.
                egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR },
            );
            doc.caches.thumbs.insert(done.page, handle);
            self.render_state.render_stats.previewed += 1;
        }
    }
}
