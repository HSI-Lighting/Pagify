//! Page pictures and the rail's thumbnails, drawn by hayro on threads of their
//! own ([`pagify_shell::preview`]) instead of by the render worker's one PDFium
//! thread, where a heavy page held the engine for what it cost while everything
//! else waited (reported from use, 2026-11-05: a CAD set's rail took 4.7 s of
//! PDFium; its pages 2.9 s each against hayro's 0.6 s).
//!
//! **Where hayro draws a page as PDFium does** — the owner's decision of
//! 2026-11-05, after the comparison of the two on their own PDFs (architecture
//! doc §51). A page hayro may draw differently ([`Preview::differs_because`]) is
//! shown in hayro's picture meanwhile and PDFium's replaces it, under the same
//! key; from then on its pictures are PDFium's. Its thumbnails stay hayro's (the
//! owner's 0.1.116 choice). With "Exact" on ([`crate::PrefsState::exact_pages`])
//! every picture is PDFium's. Edit Text's textless picture, printing, export and
//! every read and edit are PDFium's always.
//!
//! The threads share one hayro parse of each document state, read from the
//! session's bytes as it stands. A picture is stamped with the document's render
//! epoch, like a render, and one for a page as it no longer is is dropped. Where
//! hayro cannot read a document or draw a page, that picture is asked of PDFium.

use pagify_shell::preview::{turned, Difference, Preview};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::{RenderDone, RenderJob, THUMB_STEP_BASE};

/// What came back.
pub(crate) struct PreviewDone {
    /// The picture as the render worker would have sent it; an `Err` image
    /// where hayro could not draw it.
    pub(crate) done: RenderDone,
    /// The same picture, to ask of PDFium: hayro could not draw it (`None`), or
    /// this is a page hayro may draw differently, and why.
    pub(crate) exactly: Option<(RenderJob, Option<Difference>)>,
}

/// Each document as hayro last read it: the state, and the parse — `None`
/// when hayro could not read that state.
type Read = Mutex<HashMap<u64, (u64, Option<Arc<Preview>>)>>;

/// The threads that draw with hayro.
pub(crate) struct PreviewWorker {
    jobs: Sender<RenderJob>,
    pub(crate) done: Receiver<PreviewDone>,
    /// The render worker's keys: `(document, page, slot kind)` to the
    /// `(step, rotation, crop)` asked for and not back yet.
    pub(crate) in_flight: HashMap<(u64, usize, u8), (u32, u8, Option<pdf_core::document::Rect>)>,
    /// What hayro could not draw, as `(document, epoch, page)`: asked of PDFium.
    pub(crate) refused: HashSet<(u64, u64, usize)>,
    /// Pages hayro may draw differently, as `(document, epoch, page)`: their
    /// pictures, not their thumbnails, asked of PDFium.
    pub(crate) differs: HashSet<(u64, u64, usize)>,
    /// `(document, page)` already written to the session log as PDFium's.
    logged: HashSet<(u64, usize)>,
}

impl PreviewWorker {
    pub(crate) fn start(ctx: &egui::Context) -> Option<Self> {
        let (jobs, queue) = channel::<RenderJob>();
        let (reply, done) = channel::<PreviewDone>();
        let queue = Arc::new(Mutex::new(queue));
        let read: Arc<Read> = Arc::default();
        // As PdfCraft sizes its render pool: a core left for everything else.
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 8) - 1;
        for n in 0..threads {
            let (queue, reply, read, ctx) = (queue.clone(), reply.clone(), read.clone(), ctx.clone());
            std::thread::Builder::new()
                .name(format!("pagify-preview-{n}"))
                // ponytail: first asked, first drawn, every ask drawn — a zoom
                // that moves on still has its older sizes drawn; give the
                // threads the render worker's newest-first queue if that shows.
                .spawn(move || loop {
                    let job = match queue.lock().map(|q| q.recv()) {
                        Ok(Ok(job)) => job,
                        _ => return,
                    };
                    // A newer state of the document has been read since this was
                    // asked: the page as it was is no use to anyone.
                    let Some(preview) = preview_of(&read, &job) else { continue };
                    let started = std::time::Instant::now();
                    let raster = preview.as_ref().and_then(|p| match job.crop {
                        Some(crop) => p.render_region(job.page, crop, job.scale),
                        None => p.render_scaled(job.page, job.scale).map(|r| turned(r, job.rotation)),
                    });
                    let image = raster.map(|r| crate::page_to_image(&r)).ok_or_else(|| "hayro could not draw it".to_string());
                    let reason = match &preview {
                        Some(p) if image.is_ok() && job.step < THUMB_STEP_BASE => p.differs_because(job.page),
                        _ => None,
                    };
                    let refused = image.is_err();
                    let done = RenderDone {
                        doc: job.doc,
                        epoch: job.epoch,
                        page: job.page,
                        step: job.step,
                        rotation: job.rotation,
                        crop: job.crop,
                        hide: None,
                        took: started.elapsed(),
                        image,
                    };
                    let exactly = (refused || reason.is_some()).then_some((job, reason));
                    if reply.send(PreviewDone { done, exactly }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                })
                .ok()?;
        }
        Some(PreviewWorker {
            jobs,
            done,
            in_flight: HashMap::new(),
            refused: HashSet::new(),
            differs: HashSet::new(),
            logged: HashSet::new(),
        })
    }
}

/// The parse a job is drawn from: read once a state, by whichever thread gets
/// there first, the others waiting for it. `Some(None)` when hayro cannot read
/// that state; `None` when the job is for a state older than the one read.
fn preview_of(read: &Read, job: &RenderJob) -> Option<Option<Arc<Preview>>> {
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
    /// Offer a render to hayro. `Err(job)` when hayro is not to draw it — the
    /// editor's textless picture, "Exact" on, a page hayro could not draw or may
    /// draw differently (its thumbnail excepted), or no threads to draw on — for
    /// the caller to ask PDFium.
    pub(crate) fn ask_for_preview(&mut self, ctx: &egui::Context, job: RenderJob) -> Result<(), RenderJob> {
        if job.hide.is_some() || self.prefs_state.exact_pages {
            return Err(job);
        }
        if self.render_state.previews.is_none() {
            self.render_state.previews = PreviewWorker::start(ctx);
        }
        let Some(worker) = self.render_state.previews.as_mut() else { return Err(job) };
        let state = (job.doc, job.epoch, job.page);
        if worker.refused.contains(&state) || (job.step < THUMB_STEP_BASE && worker.differs.contains(&state)) {
            return Err(job);
        }
        let slot = (job.doc, job.page, crate::slot_kind(&job.crop, job.step));
        let want = (job.step, job.rotation as u8, job.crop);
        if worker.in_flight.get(&slot) == Some(&want) {
            return Ok(());
        }
        match worker.jobs.send(job) {
            Ok(()) => {
                worker.in_flight.insert(slot, want);
                Ok(())
            }
            // The threads have gone: PDFium draws everything again.
            Err(std::sync::mpsc::SendError(job)) => {
                self.render_state.previews = None;
                Err(job)
            }
        }
    }

    /// The pictures hayro has drawn, filed as the render worker's are — once a
    /// frame. One hayro could not draw, or of a page it may draw differently, is
    /// asked of PDFium, whose picture replaces hayro's.
    pub(crate) fn collect_previews(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.render_state.previews.as_mut() else { return };
        let mut arrived = Vec::new();
        while let Ok(back) = worker.done.try_recv() {
            let d = &back.done;
            let slot = (d.doc, d.page, crate::slot_kind(&d.crop, d.step));
            if worker.in_flight.get(&slot) == Some(&(d.step, d.rotation as u8, d.crop)) {
                worker.in_flight.remove(&slot);
            }
            let state = (d.doc, d.epoch, d.page);
            match &back.exactly {
                Some((_, None)) => {
                    worker.refused.insert(state);
                }
                Some((_, Some(_))) => {
                    worker.differs.insert(state);
                }
                None => {}
            }
            arrived.push(back);
        }
        for PreviewDone { done, exactly } in arrived {
            let (doc, page) = (done.doc, done.page);
            if done.image.is_ok() && self.file_picture(ctx, done) {
                self.render_state.render_stats.previewed += 1;
            }
            let Some((job, reason)) = exactly else { continue };
            if let Some(reason) = reason {
                let first = self.render_state.previews.as_mut().is_some_and(|w| w.logged.insert((doc, page)));
                if first {
                    self.recording_state.session_log.record("render", &format!("page {} drawn by PDFium: {}", page + 1, reason.reason()));
                }
            }
            self.ask_for_render(ctx, job);
        }
    }
}
