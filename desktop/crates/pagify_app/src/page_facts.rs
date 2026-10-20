//! What the pointer needs to know about a page, read off the UI thread.
//!
//! **Reported from use: a drawing of 700,000 lines froze the window.** Asking the
//! engine whether a page has a link, an annotation or any words under the pointer
//! makes PDFium parse the whole page first — 0.6 s for the annotations, 1.2 s for the
//! links (it parses twice), 0.7 s for the words, on one of that drawing's pages — and
//! a page being drawn holds the one lock over PDFium for 2-3 s besides. The UI thread
//! made all of those calls the first time the pointer crossed a page, and again each
//! time it came back to it.
//!
//! Here they are made by a thread of their own, once per page and state of the page,
//! and the frame only ever looks at what has come back. Until it has, the pointer is
//! over a page that has nothing under it, which for a moment it may be wrong about
//! and for a light page is a frame or two.
//!
//! Only with the render worker on ([`crate::RenderState::async_render`]): the tests,
//! which run without it, read what they ask for where they ask for it, as before.

use pagify_shell::page_space::AppPoint;
use pagify_shell::reader::Characters;
use pagify_shell::session::Session;
use pdf_core::document::{Annotation, IndexedAnnotation, InternalLink, PageScale, Rect};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Everything about one page the hover needs, as the page was when it was read.
pub(crate) struct PageFacts {
    /// The document's `render_epoch` and the session's undo generation at the
    /// time: good only while both still are.
    pub(crate) epoch: u64,
    pub(crate) generation: u64,
    /// How many objects the page has, how many of them text — see
    /// [`crate::PagifyApp::page_weight`].
    pub(crate) weight: Option<PageScale>,
    /// Each annotation that has a rectangle to hit, by its listing number.
    foreign: Vec<(usize, Vec<Rect>)>,
    /// The listing numbers of those that are links.
    link_numbers: HashSet<usize>,
    /// Links to another page of this document.
    links: Vec<InternalLink>,
    /// The page's words, one box each.
    pub(crate) characters: Option<Characters>,
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left.min(r.right) && x <= r.left.max(r.right) && y >= r.top.min(r.bottom) && y <= r.top.max(r.bottom)
}

/// The rectangles of every annotation a pointer can be over, with its listing
/// number — what [`crate::PagifyApp::foreign_marks`] keeps, from the same list.
pub(crate) fn foreign_rects(marks: &[IndexedAnnotation]) -> Vec<(usize, Vec<Rect>)> {
    marks
        .iter()
        .enumerate()
        .filter_map(|(n, m)| {
            let rects = match &m.annotation {
                Annotation::Highlight { rects, .. }
                | Annotation::Underline { rects, .. }
                | Annotation::StrikeOut { rects, .. }
                | Annotation::Squiggly { rects, .. } => rects.clone(),
                Annotation::Note { rect, .. } => vec![*rect],
                Annotation::Image { rect, .. } => vec![*rect],
                Annotation::Link { rect, .. } => vec![*rect],
                // Ink and Fill are this engine's own markup, tracked live in
                // `markup::Layer` rather than hit-tested here; text is page content
                // rather than an annotation.
                Annotation::Ink { .. } | Annotation::Text { .. } | Annotation::Fill { .. } => return None,
            };
            Some((n + 1, rects))
        })
        .collect()
}

impl PageFacts {
    /// Read `page` — each part separately, so the engine is let go of between them
    /// and a render waiting for it gets in. **Slow on a heavy page; never call it on
    /// the UI thread.**
    fn read(session: &Session, page: usize, epoch: u64, generation: u64) -> Self {
        let weight = session.page_scale(page).ok();
        let marks = session.annotations(page).unwrap_or_default();
        let link_numbers = marks
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.annotation, Annotation::Link { .. }))
            .map(|(n, _)| n + 1)
            .collect();
        let foreign = foreign_rects(&marks);
        let links = session.internal_links(page).unwrap_or_default();
        let characters = session.characters(page).ok();
        PageFacts { epoch, generation, weight, foreign, link_numbers, links, characters }
    }

    /// Whether a link — a web address or another page — is under `at`. The same
    /// answer a click gets: the first annotation under the point, and only if that
    /// one is a link, else a link to a page.
    pub(crate) fn link_at(&self, at: AppPoint) -> bool {
        let (x, y) = (at.x as f32, at.y as f32);
        let first = self.foreign.iter().find(|(_, rects)| rects.iter().any(|r| contains(r, x, y)));
        first.is_some_and(|(n, _)| self.link_numbers.contains(n))
            || self.links.iter().any(|l| contains(&l.rect, x, y))
    }

    /// Whether a word is under `at`.
    pub(crate) fn word_at(&self, at: AppPoint) -> bool {
        self.characters.as_ref().is_some_and(|c| c.hit(at.x as f32, at.y as f32).is_some())
    }
}

/// A page to read, handed to the facts worker.
pub(crate) struct FactsJob {
    pub(crate) doc: u64,
    pub(crate) epoch: u64,
    pub(crate) generation: u64,
    pub(crate) page: usize,
    pub(crate) session: Arc<Session>,
}

pub(crate) struct FactsDone {
    pub(crate) doc: u64,
    pub(crate) facts: PageFacts,
    pub(crate) page: usize,
}

/// The thread that reads pages for [`PageFacts`].
pub(crate) struct FactsWorker {
    jobs: Sender<FactsJob>,
    pub(crate) done: Receiver<FactsDone>,
    /// `(document, page)` to the `(epoch, generation)` asked for and not back yet.
    pub(crate) in_flight: HashMap<(u64, usize), (u64, u64)>,
}

impl FactsWorker {
    pub(crate) fn start(ctx: &egui::Context) -> Option<Self> {
        let (jobs, queue) = channel::<FactsJob>();
        let (reply, done) = channel::<FactsDone>();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("pagify-facts".into())
            .spawn(move || {
                // **Newest first.** A pointer sweeping over the strip asks for a page
                // per few frames and a heavy one takes seconds, so by the time the
                // thread is free the page it is over now is the last one asked for.
                let mut pending: Vec<FactsJob> = Vec::new();
                loop {
                    if pending.is_empty() {
                        match queue.recv() {
                            Ok(job) => pending.push(job),
                            Err(_) => return,
                        }
                    }
                    pending.extend(queue.try_iter());
                    let Some(job) = pending.pop() else { continue };
                    // The same page asked again for an older state: not worth reading.
                    pending.retain(|p| !(p.doc == job.doc && p.page == job.page));
                    let facts = PageFacts::read(&job.session, job.page, job.epoch, job.generation);
                    if reply.send(FactsDone { doc: job.doc, facts, page: job.page }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(FactsWorker { jobs, done, in_flight: HashMap::new() })
    }

    /// Ask for a page — unless exactly that is already on its way.
    pub(crate) fn ask(&mut self, job: FactsJob) -> bool {
        let key = (job.doc, job.page);
        let want = (job.epoch, job.generation);
        if self.in_flight.get(&key) == Some(&want) {
            return true;
        }
        if self.jobs.send(job).is_ok() {
            self.in_flight.insert(key, want);
            true
        } else {
            false
        }
    }
}

impl crate::PagifyApp {
    /// What the pointer needs to know about `page`, **if it has been read** under the
    /// page's present state; asked of the facts worker if not, and `None` meanwhile.
    /// Never waits for the engine. `None` also when there is no worker to ask
    /// (the tests): those read where they need to.
    pub(crate) fn page_facts(&mut self, ctx: &egui::Context, page: usize) -> Option<Arc<PageFacts>> {
        if !self.render_state.async_render {
            return None;
        }
        let doc = self.tab().doc.as_ref()?;
        let (epoch, generation) = (doc.render_epoch, doc.session.undo_generation());
        if let Some(held) = doc.caches.facts.get(&page) {
            if held.epoch == epoch && held.generation == generation {
                return Some(held.clone());
            }
        }
        let job = FactsJob { doc: doc.id, epoch, generation, page, session: doc.session.clone() };
        if self.render_state.facts.is_none() {
            self.render_state.facts = FactsWorker::start(ctx);
        }
        let asked = self.render_state.facts.as_mut().is_some_and(|w| w.ask(job));
        if !asked {
            // The thread is gone: back to reading where it is needed.
            self.render_state.facts = None;
            self.render_state.async_render = false;
        }
        None
    }

    /// The pages that have come back from the facts worker, filed — once a frame.
    /// One that no longer describes its page is dropped.
    pub(crate) fn collect_facts(&mut self) {
        let Some(worker) = self.render_state.facts.as_mut() else { return };
        let mut arrived = Vec::new();
        while let Ok(done) = worker.done.try_recv() {
            let key = (done.doc, done.page);
            if worker.in_flight.get(&key) == Some(&(done.facts.epoch, done.facts.generation)) {
                worker.in_flight.remove(&key);
            }
            arrived.push(done);
        }
        for done in arrived {
            let current = self
                .tabs
                .iter_mut()
                .filter_map(|t| t.doc.as_mut())
                .find(|d| d.id == done.doc && d.render_epoch == done.facts.epoch && d.session.undo_generation() == done.facts.generation);
            if let Some(doc) = current {
                doc.caches.facts.insert(done.page, Arc::new(done.facts));
            }
        }
    }
}
