//! Everything a `Doc` knows about its own pages that is cheaper to keep than
//! to ask the engine for again — the review's Phase 3 cache registry.
//!
//! **One struct, one function that empties it.** Before this, eight of these
//! fields lived loose on `Doc` and were already funnelled through
//! [`crate::Doc::rendered_is_stale`] — see that method's own doc for the bug
//! (a locked page still showing in the thumbnail strip) that made a single
//! clearing point necessary in the first place. The other five —
//! `text`/`layers`/`foreign`/`internal_links`/`drawn_words` — used to live on
//! `DocTab` instead, each cleared by hand at every one of the ~45 places an
//! edit could make it stale, every one of them paired with a
//! `doc.rendered_is_stale()` call a line or two above. Moving them in here
//! means that pairing is no longer something a new edit path has to remember:
//! `rendered_is_stale` reaches all thirteen at once, and the ~45 individual
//! clears that used to shadow it are simply gone.
//!
//! `internal_links` and `drawn_words` do not actually need any of this: both
//! are stamped with `(page, render_epoch, undo_generation)` and already
//! refuse themselves on a stale stamp, the same way the eight original
//! fields do. They are kept here anyway, for the same reason `page_blocks`'s
//! own doc gives for clearing a self-stamped field eagerly too: one place to
//! look, not thirteen minus two.

use crate::{DetailTile, PageBlocks, PageRaster};
use pagify_shell::page_space::AppPoint;

#[derive(Default)]
pub(crate) struct DocCaches {
    /// One texture per (page, quantised scale). Bounded by eviction of pages
    /// that have scrolled out — see [`Self::evict_outside`] — and, per page,
    /// to `SCALES_KEPT_PER_PAGE` scales.
    pub(super) textures: std::collections::HashMap<(usize, u32, u8), egui::TextureHandle>,
    pub(super) thumbs: std::collections::HashMap<usize, egui::TextureHandle>,
    /// A sharper crop of whatever's on screen, layered over the capped
    /// whole-page texture when that cap would otherwise leave a zoomed-in
    /// detail blurry. One at a time: only the page actually being looked at
    /// closely needs this, not every page in the document.
    pub(super) detail: Option<DetailTile>,
    /// What is sealed on each page, asked of the engine once and kept —
    /// drawing the lock badges asked every frame, for every page in view,
    /// and every ask takes the one lock that covers all use of PDFium.
    pub(super) locked: std::cell::RefCell<std::collections::HashMap<usize, Vec<pdf_core::document::LockedItem>>>,
    /// The one page Edit Text last looked at, read and detected into
    /// paragraphs — see [`crate::PagifyApp::page_blocks`].
    pub(super) page_blocks: std::cell::RefCell<Option<std::rc::Rc<PageBlocks>>>,
    /// The page last rendered small to sample the colour behind a paragraph
    /// from — see [`crate::PagifyApp::page_raster_for_sampling`].
    pub(super) sampling: std::cell::RefCell<Option<((usize, u64, u64), std::rc::Rc<PageRaster>)>>,
    /// What the page Edit Text last looked at is made of — see
    /// [`crate::PagifyApp::page_weight`].
    pub(super) weight: std::cell::Cell<Option<((usize, u64, u64), pdf_core::document::PageScale)>>,
    /// The page of text objects a click on a heavy page is resolved against
    /// — see [`crate::PagifyApp::light_page`].
    pub(super) rect_page: std::cell::RefCell<Option<(bool, std::rc::Rc<PageBlocks>)>>,
    /// The current page's characters — see [`crate::PagifyApp::characters`].
    pub(super) text: Option<(usize, pagify_shell::reader::Characters)>,
    /// What the current page draws, bottom first — see
    /// [`crate::PagifyApp::layers_on`].
    pub(super) layers: Option<(usize, Vec<pdf_core::document::DrawnObject>)>,
    /// The rectangles of every annotation on a page — see
    /// [`crate::PagifyApp::foreign_marks`].
    pub(super) foreign: Option<(usize, Vec<(usize, Vec<pdf_core::document::Rect>)>)>,
    /// The links on a page that go to another page — see
    /// [`crate::PagifyApp::internal_link_at`].
    pub(super) internal_links: Option<(usize, u64, Vec<pdf_core::document::InternalLink>)>,
    /// The words a page draws rather than writes — see
    /// [`crate::PagifyApp::drawn_words_on`].
    pub(super) drawn_words: Option<((usize, u64, u64), Vec<pdf_core::document::RecognisedWord>)>,
}

impl DocCaches {
    /// Throw away everything already read or drawn from this document — the
    /// one function every edit path calls through
    /// [`crate::Doc::rendered_is_stale`], so the next one added is right by
    /// default.
    pub(super) fn invalidate(&mut self) {
        self.textures.clear();
        self.thumbs.clear();
        self.detail = None;
        self.locked.get_mut().clear();
        self.page_blocks.get_mut().take();
        self.sampling.get_mut().take();
        self.weight.set(None);
        self.rect_page.get_mut().take();
        self.text = None;
        self.layers = None;
        self.foreign = None;
        self.internal_links = None;
        self.drawn_words = None;
    }

    /// Drop the whole-page textures and the detail crop for pages that have
    /// scrolled well clear of the window — a fifty-page document at full
    /// zoom would otherwise hold fifty full-size rasters on the GPU.
    ///
    /// **Not invalidation.** This runs every frame, from where it is
    /// scrolled to, not from anything that changed on the page — a page
    /// coming back into view still has a perfectly good texture and must not
    /// be made to render itself again.
    pub(super) fn evict_outside(&mut self, keep_from: usize, keep_to: usize) {
        self.textures.retain(|(page, _, _), _| *page >= keep_from && *page < keep_to);
        if self.detail.as_ref().is_some_and(|d| d.page < keep_from || d.page >= keep_to) {
            self.detail = None;
        }
    }
}

impl crate::Doc {
    /// Throw away everything already drawn from this document.
    ///
    /// **Every cache, always.** Reported from use: a locked page still showed
    /// its contents in the thumbnail strip. Thirteen places cleared `textures`
    /// after an edit and two cleared `thumbs`, so almost every edit left a
    /// stale thumbnail — and for a lock or a redaction that is not a cosmetic
    /// lag, it is the hidden content still on screen.
    ///
    /// One method rather than a call at each site, because the next cache
    /// added will be cleared by it without anyone having to remember to.
    pub(crate) fn rendered_is_stale(&mut self) {
        self.caches.invalidate();
        // And whatever the render worker is part-way through: it is a picture
        // of the page as it was.
        self.render_epoch += 1;
    }
}

impl crate::PagifyApp {
    /// This page's characters, extracted once and kept.
    pub(crate) fn characters(&mut self, page: usize) -> Option<&pagify_shell::reader::Characters> {
        let doc = self.tab_mut().doc.as_mut()?;
        if doc.caches.text.as_ref().map(|(p, _)| *p) != Some(page) {
            let chars = doc.session.characters(page).ok()?;
            doc.caches.text = Some((page, chars));
        }
        doc.caches.text.as_ref().map(|(_, chars)| chars)
    }

    /// Clear just the drawing-order cache, where something changed it
    /// without otherwise making the document's own caches stale — a drag
    /// that moved nothing, the Layers panel being opened to show it fresh.
    pub(crate) fn forget_layers(&mut self) {
        if let Some(doc) = self.tab_mut().doc.as_mut() {
            doc.caches.layers = None;
        }
    }

    /// What this page draws, bottom first — read fresh when the page changes.
    ///
    /// **Not cached across an edit.** Restacking rewrites the content stream
    /// and PDFium renumbers the page's objects afterwards, so a list held from
    /// before would name things that have since moved.
    pub(crate) fn layers_on(&mut self, page: usize) -> &[pdf_core::document::DrawnObject] {
        let Some(doc) = self.tab_mut().doc.as_mut() else { return &[] };
        if doc.caches.layers.as_ref().map(|(p, _)| *p) != Some(page) {
            let found = doc.session.drawn_objects(page).ok().unwrap_or_default();
            doc.caches.layers = Some((page, found));
        }
        doc.caches.layers.as_ref().map(|(_, l)| l.as_slice()).unwrap_or(&[])
    }

    /// The rectangles of every annotation on a page, with its listing number.
    ///
    /// Rebuilt when the page changes, not on every pointer move: reading
    /// annotations goes through PDFium and a mouse crossing a page would ask
    /// hundreds of times a second.
    pub(crate) fn foreign_marks(&mut self, page: usize) -> &[(usize, Vec<pdf_core::document::Rect>)] {
        let Some(doc) = self.tab_mut().doc.as_mut() else { return &[] };
        if doc.caches.foreign.as_ref().map(|(p, _)| *p) != Some(page) {
            use pdf_core::document::Annotation as A;
            let marks = doc.session
                .annotations(page)
                .ok()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .filter_map(|(n, m)| {
                    let rects = match &m.annotation {
                        A::Highlight { rects, .. }
                        | A::Underline { rects, .. }
                        | A::StrikeOut { rects, .. }
                        | A::Squiggly { rects, .. } => rects.clone(),
                        A::Note { rect, .. } => vec![*rect],
                        A::Image { rect, .. } => vec![*rect],
                        A::Link { rect, .. } => vec![*rect],
                        // Ink and Fill are this engine's own markup, tracked
                        // live in `markup::Layer` rather than hit-tested here;
                        // text is page content rather than an annotation.
                        A::Ink { .. } | A::Text { .. } | A::Fill { .. } => return None,
                    };
                    Some((n + 1, rects))
                })
                .collect();
            doc.caches.foreign = Some((page, marks));
        }
        doc.caches.foreign.as_ref().map(|(_, m)| m.as_slice()).unwrap_or(&[])
    }

    /// The page a link under `at` goes to, when it is one of this document's
    /// own pages — a contents list entry, a "back to the index".
    ///
    /// **Reported from use: "links inside a PDF don't work".** Only web
    /// addresses were ever followed; a link to a page was not even listed, so
    /// clicking a contents entry did nothing at all.
    pub(crate) fn internal_link_at(&mut self, page: usize, at: AppPoint) -> Option<usize> {
        let (generation, session) = {
            let doc = self.tab().doc.as_ref()?;
            (doc.session.undo_generation(), doc.session.clone())
        };
        let stale =
            !matches!(&self.tab().doc.as_ref()?.caches.internal_links, Some((p, g, _)) if *p == page && *g == generation);
        if stale {
            let links = session.internal_links(page).unwrap_or_default();
            if let Some(doc) = self.tab_mut().doc.as_mut() {
                doc.caches.internal_links = Some((page, generation, links));
            }
        }
        let (x, y) = (at.x as f32, at.y as f32);
        let (_, _, links) = self.tab().doc.as_ref()?.caches.internal_links.as_ref()?;
        // The smallest link under the point: a button inside a larger banner
        // link is the one that was meant.
        links
            .iter()
            .filter(|l| x >= l.rect.left && x <= l.rect.right && y >= l.rect.top && y <= l.rect.bottom)
            .min_by(|a, b| {
                let area = |l: &pdf_core::document::InternalLink| {
                    (l.rect.right - l.rect.left) * (l.rect.bottom - l.rect.top)
                };
                area(a).total_cmp(&area(b))
            })
            .map(|l| l.page)
    }

    /// The words this page *draws*, recognised without changing it.
    ///
    /// **Deliberately not `extracttext`.** That writes a transparent text layer
    /// over the artwork so the page can be searched — and writing it re-emits
    /// the page, which puts its paths inside a form. Objects nested in a form
    /// cannot be taken off by a redaction at all, so extracting first is
    /// precisely what makes a drawn word unremovable afterwards. Measured: the
    /// same word, in the same box, came off a freshly opened document and was
    /// refused on one that had been extracted.
    ///
    /// So this asks the recogniser for the words and keeps them here, in
    /// memory. The page is not touched, and stays as removable as it was.
    ///
    /// **Good only under the stamp it was made under.** Kept by the page
    /// index alone, it went on listing a word a redaction had taken off the
    /// page, and opened an editor for it — and answered for another page
    /// after a page was deleted or moved.
    pub(crate) fn drawn_words_on(&mut self, page: usize) -> &[pdf_core::document::RecognisedWord] {
        let stamp = self.doc_stamp(page);
        let stale = self.tab_mut().doc.as_ref().map_or(true, |doc| {
            doc.caches.drawn_words.as_ref().map(|(at, _)| *at) != Some(stamp)
        });
        if stale {
            let faces = self.outlined_font_bytes();
            let borrowed: Vec<&[u8]> = faces.iter().map(Vec::as_slice).collect();
            let found = self.tab_mut()
                .doc
                .as_ref()
                .and_then(|d| d.session.recognise_drawn_words(page, &borrowed).ok())
                .flatten()
                .unwrap_or_default();
            // **Not filtered for plausibility, deliberately.** Matching shapes
            // against a face the document was not set in produces fragments —
            // measured on a real report: 44 "words", the longest `000`. But the
            // same is true of a *correct* match on a page the recogniser splits
            // finely: `extru`, `Th`, `us` are what a good read of a real
            // fixture looks like. No statistic told the two apart.
            //
            // So the judgement is left where it can actually be made: the
            // editor opens holding the word as read, and somebody who sees
            // `000` presses Escape. Nothing changes until it is applied.
            if let Some(doc) = self.tab_mut().doc.as_mut() {
                doc.caches.drawn_words = Some((stamp, found));
            }
        }
        self.tab_mut().doc.as_ref().and_then(|d| d.caches.drawn_words.as_ref()).map(|(_, words)| words.as_slice()).unwrap_or(&[])
    }

    /// The one page Edit Text last looked at, read and detected into
    /// paragraphs, with whether it was a cache hit.
    ///
    /// **Built at the first click on a page, never when the tool is armed**
    /// (arming has to stay instant), and good only while nothing under it
    /// moved: it is stamped with this document's `render_epoch` and the
    /// session's undo generation and refused when either differs, and
    /// [`crate::Doc::rendered_is_stale`] — the one place every page-changing
    /// path already goes through — empties it. One page at a time: a click
    /// is on one page, and a second page's text is a second read.
    ///
    /// Everything the click needs about the page (words, fonts, shapes)
    /// comes out of this one [`pagify_shell::Session::page_text_snapshot`]
    /// call — one registry lock — instead of the half dozen separate calls
    /// the old walk made.
    pub(crate) fn page_blocks(&self, page: usize) -> Result<(std::rc::Rc<PageBlocks>, bool), String> {
        let Some(doc) = self.tab().doc.as_ref() else { return Err("nothing open.".into()) };
        let (epoch, generation) = (doc.render_epoch, doc.session.undo_generation());
        if let Some(held) = doc.caches.page_blocks.borrow().as_ref() {
            if held.page == page && held.epoch == epoch && held.generation == generation {
                return Ok((held.clone(), true));
            }
        }
        #[cfg(test)]
        if crate::tests_support::SNAPSHOT_FAILS.with(|fails| fails.get()) {
            return Err("test: the page's text is unavailable".into());
        }
        let read = std::time::Instant::now();
        let snapshot = doc.session.page_text_snapshot(page).map_err(|e| format!("{e}"))?;
        let read_ms = read.elapsed().as_secs_f32() * 1000.0;
        let mut built = crate::block_input::build_page_blocks(page, epoch, generation, snapshot);
        // The detector's own time plus PDFium's.
        built.build_ms += read_ms;
        let built = std::rc::Rc::new(built);
        *doc.caches.page_blocks.borrow_mut() = Some(built.clone());
        Ok((built, false))
    }

    /// What the page is made of — how many objects, how many of them text —
    /// and so whether it is **heavy**: counted without reading any of it,
    /// **once per state of the page** (`(page, render epoch, undo
    /// generation)`, as for [`Self::page_blocks`]). `None` when the page
    /// cannot be counted (it is not there), which is not a reason to refuse
    /// anything: the click goes on, and the read that follows says what is
    /// wrong.
    pub(crate) fn page_weight(&self, page: usize) -> Option<pdf_core::document::PageScale> {
        let doc = self.tab().doc.as_ref()?;
        let stamp = (page, doc.render_epoch, doc.session.undo_generation());
        if let Some((held, weight)) = doc.caches.weight.get() {
            if held == stamp {
                return Some(weight);
            }
        }
        let weight = doc.session.page_scale(page).ok()?;
        doc.caches.weight.set(Some((stamp, weight)));
        Some(weight)
    }

    /// **The page's text objects and nothing else** — where each one is and,
    /// when `with_words`, what it says — for a click that must not read the
    /// whole page for paragraphs: no fonts, no shapes, no detector. Made
    /// **once per state of the page**, under the same key as
    /// [`Self::page_blocks`]: it opens the page and walks every object on
    /// it, which on a very large page is the whole of what a click costs,
    /// and a click after the first then costs nothing at all.
    ///
    /// Read the heavy way (`with_words`) it is one pass over the page's text
    /// — linear in the number of words — rather than a page open for every
    /// run asked for.
    pub(crate) fn light_page(&self, page: usize, with_words: bool) -> Result<std::rc::Rc<PageBlocks>, String> {
        let Some(doc) = self.tab().doc.as_ref() else { return Err("nothing open.".into()) };
        let (epoch, generation) = (doc.render_epoch, doc.session.undo_generation());
        let held = doc.caches.rect_page.borrow().as_ref().and_then(|(has_words, held)| {
            let good = (*has_words || !with_words) && held.page == page && held.epoch == epoch && held.generation == generation;
            good.then(|| held.clone())
        });
        if let Some(held) = held {
            return Ok(held);
        }
        let started = std::time::Instant::now();
        let runs: Vec<pdf_core::document::TextRun> = if with_words {
            doc.session.text_runs(page).map_err(|e| format!("{e}"))?
        } else {
            doc.session
                .text_run_rects(page)
                .map_err(|e| format!("{e}"))?
                .into_iter()
                .map(|(object, rect)| pdf_core::document::TextRun {
                    object,
                    text: String::new(),
                    rect,
                    origin: pdf_core::document::Point { x: rect.left, y: rect.bottom },
                    size: 0.0,
                    color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
                })
                .collect()
        };
        // Only what has an area can be clicked on, once each.
        let mut by_object = std::collections::HashMap::with_capacity(runs.len());
        for run in runs {
            if (run.rect.right - run.rect.left).abs() > 0.0 && (run.rect.bottom - run.rect.top).abs() > 0.0 {
                by_object.entry(run.object).or_insert(run);
            }
        }
        let built = std::rc::Rc::new(PageBlocks {
            page,
            epoch,
            generation,
            runs: by_object,
            faces: std::collections::HashMap::new(),
            styles: std::collections::HashMap::new(),
            shapes: Vec::new(),
            frags: Vec::new(),
            blocks: Vec::new(),
            by_object: std::collections::HashMap::new(),
            twins: std::collections::HashMap::new(),
            excluded: Default::default(),
            build_ms: started.elapsed().as_secs_f32() * 1000.0,
            detect_ms: 0.0,
        });
        *doc.caches.rect_page.borrow_mut() = Some((with_words, built.clone()));
        Ok(built)
    }

    /// Render the page once at [`Self::BACKGROUND_SAMPLE_SCALE`], for
    /// [`Self::background_at`] to sample from — the render half of
    /// [`Self::page_behind`], split out for a caller that already has (or
    /// wants to keep) the raster separately from the one rect it samples.
    ///
    /// **Made once per state of the page, not once per click.** Every click
    /// on a paragraph of one page wanted the same picture, and a render is
    /// the bulk of what a click costs once the page has been read, under the
    /// one lock that covers all use of PDFium. It is kept on the document
    /// under the same key as [`Self::page_blocks`] — `(page, render epoch,
    /// undo generation)` — and emptied in the same place,
    /// [`crate::Doc::rendered_is_stale`], so what it samples is always the
    /// page as it is now.
    pub(crate) fn page_raster_for_sampling(&self, page: usize) -> Option<std::rc::Rc<PageRaster>> {
        let doc = self.tab().doc.as_ref()?;
        let key = (page, doc.render_epoch, doc.session.undo_generation());
        if let Some((held, raster)) = doc.caches.sampling.borrow().as_ref() {
            if *held == key {
                return Some(raster.clone());
            }
        }
        let raster = std::rc::Rc::new(doc.session.render_page(page, Self::BACKGROUND_SAMPLE_SCALE).ok()?);
        *doc.caches.sampling.borrow_mut() = Some((key, raster.clone()));
        Some(raster)
    }
}
