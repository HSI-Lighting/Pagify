//! An open document, and the rasterised pages it hands upward.
//!
//! ## What "no bridge on desktop" does and does not delete
//!
//! The build plan's §5.3 is right that desktop needs no JNI and no C ABI: this
//! is Rust calling Rust, so a page arrives as a buffer this process already
//! owns. No JSON on the way through, no ownership contract across a language
//! boundary, no `char *` to forget to free.
//!
//! It is wrong about one thing, and the cost of believing it is a crash. The
//! plan reads `pdf_core::registry` as a bridge artefact — "no handle registry
//! to leak" — but the registry is doing **two** jobs, and only one of them is
//! about language boundaries:
//!
//! 1. Opaque `i64` handles, so Kotlin and Swift can name a document. Desktop
//!    genuinely does not need this.
//! 2. **Serialising every PDFium access in the process.** PDFium recycles
//!    document and page addresses freely, and `pdfium-render` keys a
//!    process-global page-index cache on those raw addresses with no purge on
//!    close — so an open racing a render can be answered with *another
//!    document's page*. `registry.rs` measured it: unserialised, 771 of 800
//!    opens failed, and 3 of the 29 reads that got through returned the wrong
//!    geometry.
//!
//! Desktop needs (2) *more* than the phones do, not less — tabbed documents and
//! background prefetch are both on the roadmap, and each is the two-thread case
//! that measurement came from. Constructing a `PdfiumDocument` directly and
//! holding it outside the lock aborts the process under a parallel test runner,
//! which is how this was found.
//!
//! So the handle stays, and the leak §5.3 worried about is closed a different
//! way: within one language, a handle's lifetime can be tied to a Rust value.
//! [`Session`] owns its registry entry and releases it on drop, which is a
//! guarantee the phone bridges cannot make for themselves.

use std::path::{Path, PathBuf};

/// Where a save is staged before it replaces its target.
fn staging_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".pagify-save");
    target.with_file_name(name)
}

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;
use pdf_core::engine::{self, RenderOutcome};
use pdf_core::registry;
use pdf_core::{PixelOrder, RenderRequest, RenderTarget, Result, Rotation};

use crate::pdfium;

/// One rasterised page: tightly packed RGBA8, top-left origin.
///
/// The byte order is not incidental. `PixelOrder::Rgba` is what egui's
/// `ColorImage` consumes directly, so the app's upload is a move rather than a
/// per-pixel swizzle — `pdf_core` renders into a caller-supplied buffer whose
/// own dimensions decide the render size, which is precisely the shape the
/// toolkit wants. The zero-copy discipline the phone apps established survives.
pub struct PageRaster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// Whether this came from the page cache rather than a fresh rasterisation.
    pub from_cache: bool,
}

impl PageRaster {
    /// How many pixels are darker than `threshold` on the red channel.
    ///
    /// Crude on purpose. Its job is to answer one question — *did anything
    /// actually get drawn* — because the failure this guards against is a page
    /// that renders as a perfectly clean white rectangle and looks, to a
    /// screenshot and to a human, exactly like a blank page in the document.
    pub fn ink(&self, threshold: u8) -> usize {
        self.pixels
            .chunks_exact(4)
            .filter(|px| px[0] < threshold)
            .count()
    }
}

/// Every candidate face's glyphs, merged into one catalogue — or `None` for
/// an empty list, which is what most callers pass and must cost nothing.
fn merged_catalogue(fonts: &[&[u8]]) -> Option<pdf_core::document::glyphs::Catalogue> {
    if fonts.is_empty() {
        return None;
    }
    let mut catalogue = pdf_core::document::glyphs::Catalogue::default();
    for font in fonts {
        catalogue.extend_from_font_common(font);
    }
    Some(catalogue)
}

/// An open document. Owns its registry entry for as long as it lives.
pub struct Session {
    handle: i64,
    path: PathBuf,
}

impl Session {
    /// Open a PDF from disk, binding PDFium first if nothing has yet.
    ///
    /// The open happens *inside* the registry lock. That is the whole point:
    /// the lock has to cover the open itself, not merely the bookkeeping after
    /// it, because a document being built shares PDFium's address space with
    /// one being read.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_password(path, None)
    }

    /// Open a document that is encrypted.
    ///
    /// Separate from [`Session::open`] rather than folded into it, because a
    /// password is not part of a path and must not be handled like one: it is
    /// never stored on the session, never written to the recent list, and never
    /// recorded. It is used once, here, and dropped.
    pub fn open_with_password(path: impl AsRef<Path>, password: Option<&str>) -> Result<Self> {
        pdfium::ensure_bound();

        let path = path.as_ref().to_path_buf();
        let for_open = path.clone();
        let password = password.map(str::to_owned);
        let handle = registry::insert_with(move || {
            let document =
                PdfiumDocument::open_path(&for_open.to_string_lossy(), password.as_deref())?;
            Ok(Box::new(document) as Box<dyn Document>)
        })?;

        Ok(Session { handle, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn page_count(&self) -> Result<usize> {
        registry::with_session(self.handle, |s| Ok(s.document.page_count()))
    }

    /// The page's size in points — the height this page's [`PageSpace`] must be
    /// built from.
    ///
    /// [`PageSpace`]: crate::page_space::PageSpace
    pub fn page_size(&self, index: usize) -> Result<pdf_core::PageSize> {
        registry::with_session(self.handle, |s| s.document.page_size(index))
    }

    /// The pixel dimensions a render at `scale` will produce, without rendering.
    pub fn page_pixel_size(&self, index: usize, scale: f32) -> Result<(u32, u32)> {
        self.page_pixel_size_rotated(index, scale, Rotation::None)
    }

    /// As [`Self::page_pixel_size`], for a rotated view. A quarter turn swaps
    /// the axes, so a caller sizing a texture must ask through here rather than
    /// using the raw page size.
    pub fn page_pixel_size_rotated(
        &self,
        index: usize,
        scale: f32,
        rotation: Rotation,
    ) -> Result<(u32, u32)> {
        let request = RenderRequest { scale, rotation, ..Default::default() };
        registry::with_session(self.handle, |s| engine::page_pixel_size(s, index, &request))
    }

    /// Rasterise a page at `scale`, where 1.0 is 72 dpi.
    pub fn render_page(&self, index: usize, scale: f32) -> Result<PageRaster> {
        self.render_page_rotated(index, scale, Rotation::None)
    }

    /// Rasterise a page at `scale`, turned a quarter at a time.
    ///
    /// This is a *view* rotation: it changes what is drawn, not what is stored.
    /// Rotating the page in the document is an edit, goes through the command
    /// stack, and belongs to the Organize tab in phase 10.
    pub fn render_page_rotated(
        &self,
        index: usize,
        scale: f32,
        rotation: Rotation,
    ) -> Result<PageRaster> {
        let request = RenderRequest { scale, rotation, ..Default::default() };

        registry::with_session(self.handle, |s| {
            let (width, height) = engine::page_pixel_size(s, index, &request)?;

            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            let outcome = {
                let mut target = RenderTarget::new(
                    width,
                    height,
                    width as usize * 4,
                    PixelOrder::Rgba,
                    &mut pixels,
                )?;
                engine::render_page_into(s, index, &request, &mut target)?
            };

            Ok(PageRaster {
                width,
                height,
                pixels,
                from_cache: outcome == RenderOutcome::CacheHit,
            })
        })
    }

    /// Rasterise a page into the cache with no on-screen destination, so a
    /// later scroll onto it resolves to a copy. Phase 2 wires this to the
    /// viewport's neighbours; it is here now because the engine already has it.
    ///
    /// Returns whether it actually rasterised — `false` means the cache already
    /// held this page at this zoom. Propagated rather than swallowed because it
    /// is how a prefetch strategy can tell useful work from wasted work.
    pub fn prefetch_page(&self, index: usize, scale: f32) -> Result<bool> {
        let request = RenderRequest { scale, ..Default::default() };
        registry::with_session(self.handle, |s| engine::prefetch_page(s, index, &request))
    }

    // -- markup -------------------------------------------------------------

    /// Write a page's markup into the document: real ink for anyone to see,
    /// plus the live geometry so it is still editable when reopened.
    pub fn commit_markup(
        &self,
        page: usize,
        layer: &crate::markup::Layer,
        colour: pdf_core::document::Color,
        width: f32,
    ) -> Result<crate::commit::Committed> {
        registry::with_session(self.handle, |s| {
            crate::commit::commit_page(s, page, layer, colour, width)
        })
    }

    /// Read a page's markup back, rebuilt as a live layer.
    ///
    /// `Ok(None)` means this page has no Pagify markup — which is the ordinary
    /// case for a document nobody has marked up, and not an error.
    pub fn restore_markup(&self, page: usize) -> Result<Option<crate::markup::Layer>> {
        registry::with_session(self.handle, |s| {
            match crate::commit::restore_page(&*s.document, page) {
                Ok(Some(stored)) => Ok(Some(crate::commit::to_layer(&stored))),
                Ok(None) => Ok(None),
                Err(problem) => Err(pdf_core::PdfError::InvalidArgument(problem)),
            }
        })
    }

    /// Whether anything has been changed since the document was opened.
    pub fn is_dirty(&self) -> Result<bool> {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().map(|d| d.is_dirty()).unwrap_or(false))
        })
    }

    /// Write the document out.
    ///
    /// `incremental` appends a delta and leaves the original bytes untouched,
    /// which is what keeps an existing digital signature valid. A full copy
    /// rewrites and compacts the file and destroys every signature over it, so
    /// it is an explicit choice and never the default.
    ///
    /// ## Why this writes somewhere else first
    ///
    /// Saving over the document that is open is the ordinary case — it is what
    /// ⌘S means — and writing straight to that path **destroys it**. The file
    /// is what PDFium is reading the document from, and creating it for writing
    /// truncates it to nothing before a byte of output is produced. PDFium then
    /// refuses the save, having had its source pulled out from under it, and
    /// what is left on disk is an empty file where the document was.
    ///
    /// Measured, not reasoned about: `save` on an open fixture reduced it to
    /// zero bytes and reported "PDFium refused to save". Every earlier test
    /// passed because they all saved to a *different* path.
    ///
    /// So the output goes to a sibling temporary file and is renamed over the
    /// target once it is complete. That fixes the truncation, and it makes the
    /// save atomic as a side effect: a crash or a full disk halfway through
    /// leaves the original document exactly as it was, rather than half of a
    /// new one.
    pub fn save_to(&self, path: &Path, incremental: bool) -> Result<()> {
        // A sibling, so the rename stays on one filesystem and is therefore
        // atomic. A temporary directory could be on another volume, where
        // "rename" becomes copy-then-delete and stops being atomic at all.
        let staging = staging_path(path);

        let write = || -> Result<()> {
            let mut file = std::fs::File::create(&staging)?;
            registry::with_session(self.handle, |s| {
                pdf_core::engine::save(s, &mut file, incremental)
            })?;
            file.sync_all()?;
            Ok(())
        };

        if let Err(problem) = write() {
            // Leave nothing behind on failure. A stray `.pagify-save` beside
            // someone's document is litter at best and confusing at worst.
            let _ = std::fs::remove_file(&staging);
            return Err(problem);
        }

        std::fs::rename(&staging, path).map_err(|e| {
            let _ = std::fs::remove_file(&staging);
            pdf_core::PdfError::Io(e)
        })
    }

    // -- editing ------------------------------------------------------------

    /// Run an engine command: undoable, redoable, and — because these commands
    /// were built serialisable — recordable.
    /// Read a page with OCR, without letting a page handle escape the lock.
    ///
    /// The whole pipeline runs inside `with_session` on purpose. Rasterising is
    /// PDFium work and recognition takes about a second a page, so the
    /// temptation is to render inside the lock and recognise outside it — but
    /// that means holding a `Page` across the boundary, and every other PDFium
    /// access in this program is serialised precisely because sharing one
    /// across threads is what produced the SIGABRT this design exists to
    /// prevent.
    pub fn recognise_page(
        &self,
        index: usize,
        recogniser: &dyn pdf_core::ocr::Recogniser,
        options: &pdf_core::ocr::pipeline::Options,
    ) -> Result<pdf_core::ocr::pipeline::PageReading> {
        self.recognise_page_until(index, recogniser, options, &|| false)
    }

    /// As [`Session::recognise_page`], stopping if asked.
    pub fn recognise_page_until(
        &self,
        index: usize,
        recogniser: &dyn pdf_core::ocr::Recogniser,
        options: &pdf_core::ocr::pipeline::Options,
        stop: &dyn Fn() -> bool,
    ) -> Result<pdf_core::ocr::pipeline::PageReading> {
        registry::with_session(self.handle, |s| {
            let page = s.document.page(index)?;
            pdf_core::ocr::pipeline::read_page_until(&*page, recogniser, options, stop)
        })
    }

    /// The fast path for "Extract Text" on a page whose type was converted to
    /// outlines — real geometric matching in place of rasterising the page and
    /// running the neural recogniser on it, when a candidate face actually
    /// resolves the page's own letters.
    ///
    /// `Ok(None)` covers two cases the caller does not need to tell apart: the
    /// page is not [`pdf_core::document::PageTextKind::Outlined`] at all, or
    /// it is but none of `outlined_fonts` genuinely resolves it — see
    /// `outlined_words_are_trustworthy` on `pdf_core`'s `Page` trait for what
    /// tells "found nothing" apart from "found the wrong thing", by comparing
    /// what matched against how many type-shaped paths `classify` already
    /// counted. Either way the honest answer is the same: fall back to OCR,
    /// which this method never runs itself — that stays the caller's call,
    /// exactly as it is today.
    pub fn recognise_outlined_words_if_trustworthy(
        &self,
        index: usize,
        outlined_fonts: &[&[u8]],
    ) -> Result<Option<Vec<pdf_core::document::RecognisedWord>>> {
        let Some(catalogue) = merged_catalogue(outlined_fonts) else { return Ok(None) };
        registry::with_session(self.handle, |s| {
            let page = s.document.page(index)?;
            if page.classify()?.kind != pdf_core::document::PageTextKind::Outlined {
                return Ok(None);
            }
            let words = page.recognise_outlined_words(&catalogue)?;
            if page.outlined_words_are_trustworthy(&words)? {
                Ok(Some(words))
            } else {
                Ok(None)
            }
        })
    }

    /// The font program a run is drawn with, when the document carries one.
    pub fn run_font_data(&self, page: usize, object: usize) -> Result<Option<Vec<u8>>> {
        registry::with_session(self.handle, |s| s.document.run_font_data(page, object))
    }

    /// Shift one page object by a distance, in page points.
    pub fn move_object(
        &self,
        page: usize,
        object: usize,
        by: pdf_core::document::Point,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.move_object(page, object, by))
    }

    /// The area one page object covers.
    pub fn object_bounds(&self, page: usize, object: usize) -> Result<pdf_core::document::Rect> {
        registry::with_session(self.handle, |s| s.document.object_bounds(page, object))
    }

    /// The words a page *draws*, whatever else is on it.
    ///
    /// **Not gated on the page's classification**, unlike
    /// [`Session::recognise_outlined_words_if_trustworthy`]. `Outlined` means a
    /// page with *no* characters at all, and a page can perfectly well carry a
    /// text header over a body of type converted to outlines — measured on a
    /// real report: twenty-nine text runs and fifteen thousand curve operators
    /// on the same page, classified `Hybrid`, which that gate refuses.
    ///
    /// The caller that wants this has already established the thing the
    /// classification was standing in for: a click landed somewhere with no
    /// text under it. The question left is whether there are drawn words there,
    /// and that is what this answers — still refusing when the match is not
    /// trustworthy, which is the check that actually protects anybody.
    pub fn recognise_drawn_words(
        &self,
        index: usize,
        outlined_fonts: &[&[u8]],
    ) -> Result<Option<Vec<pdf_core::document::RecognisedWord>>> {
        let Some(catalogue) = merged_catalogue(outlined_fonts) else { return Ok(None) };
        registry::with_session(self.handle, |s| {
            let page = s.document.page(index)?;
            let words = page.recognise_outlined_words(&catalogue)?;
            if words.is_empty() {
                return Ok(None);
            }
            if page.outlined_words_are_trustworthy(&words)? {
                Ok(Some(words))
            } else {
                Ok(None)
            }
        })
    }

    /// The annotations already on a page.
    ///
    /// Everything on it, not only Pagify's own marks — a highlight another
    /// program made is still a highlight.
    pub fn annotations(
        &self,
        page: usize,
    ) -> Result<Vec<pdf_core::document::IndexedAnnotation>> {
        registry::with_session(self.handle, |s| s.document.annotations(page))
    }

    /// A page's current rotation, in quarter-turns clockwise.
    ///
    /// Needed because `SetPageRotation` is absolute and rotating is relative: a
    /// document can arrive with pages already at different angles — a landscape
    /// drawing among portrait sheets — and setting them all to one value
    /// straightens some and turns others sideways.
    pub fn page_rotation(&self, index: usize) -> Result<u8> {
        registry::with_session(self.handle, |s| {
            s.document.as_document_mut().map_or(Ok(0), |d| d.page_rotation(index))
        })
    }

    pub fn execute(&self, command: pdf_core::command::Command) -> Result<pdf_core::engine::EditState> {
        registry::with_session(self.handle, |s| pdf_core::engine::execute(s, command))
    }

    pub fn undo(&self) -> Result<(bool, pdf_core::engine::EditState)> {
        registry::with_session(self.handle, pdf_core::engine::undo)
    }

    pub fn redo(&self) -> Result<(bool, pdf_core::engine::EditState)> {
        registry::with_session(self.handle, pdf_core::engine::redo)
    }

    /// What a redaction of this rectangle would destroy, and what it could not.
    ///
    /// Runs the survey and stops. A caller shows what is in the way and, where
    /// that is an image beside the words rather than the words themselves, asks
    /// the one person who can say whether it matters.
    ///
    /// `outlined_fonts`, given, are candidate faces' own bytes — `.ttf`/`.otf`
    /// believed to include the one type-converted-to-curves on this page
    /// used. Every candidate's glyphs are merged into one
    /// [`pdf_core::document::glyphs::Catalogue`] over a broad common alphabet
    /// here, rather than asking every caller to build one: almost every
    /// caller wants exactly that alphabet, none of them should have to know
    /// this type exists to redact an ordinary page, and — measured, not
    /// assumed — merging a body face with its bold measurably helped
    /// recognition on a real fixture rather than confusing it, since the
    /// shape distance decides which candidate a letter matches regardless of
    /// which one is closer. An empty slice — the overwhelming common case,
    /// since most redactions run against ordinary text — costs nothing extra
    /// and changes nothing from before this parameter existed.
    pub fn preview_redaction(
        &self,
        page_index: usize,
        area: pdf_core::document::Rect,
        outlined_fonts: &[&[u8]],
    ) -> Result<pdf_core::document::RedactionReport> {
        let catalogue = merged_catalogue(outlined_fonts);
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("redacting this document"))?
                .preview_redaction(
                    &pdf_core::document::Redaction::new(page_index, area),
                    catalogue.as_ref(),
                )
        })
    }

    /// Hide an area, keeping a sealed copy a passcode can bring back.
    ///
    /// The passcode is borrowed and never stored — not here, not in the command
    /// history, not in the recorder. `outlined_fonts` is the same candidate list
    /// [`Session::preview_redaction`] takes, for the same reason.
    /// Hide an area, keeping a sealed copy so a passcode can bring it back.
    ///
    /// `require_complete` is the difference between the two ways a reader can
    /// ask for this, and it is not a detail:
    ///
    /// - **A dragged rectangle** means *this area*, so anything inside it that
    ///   cannot be removed — an image the words sit on — has to refuse. Leaving
    ///   it would be a document somebody believes is clear and is not.
    /// - **A text selection** means *these words*. An image underneath them was
    ///   never part of what was picked, and refusing on account of it declines a
    ///   thing the reader did not ask for. They can right-click the image and
    ///   lock that too, which is a separate decision.
    pub fn lock_area(
        &self,
        page_index: usize,
        area: pdf_core::document::Rect,
        passcode: &[u8],
        outlined_fonts: &[&[u8]],
        require_complete: bool,
    ) -> Result<pdf_core::document::RedactionReport> {
        self.lock_shapes(page_index, &[area], passcode, outlined_fonts, require_complete)
    }

    /// Lock an exact set of shapes, which is what a text selection is.
    ///
    /// **A selection over two lines is not a rectangle.** The smallest
    /// rectangle holding it also holds the head of the first line and the tail
    /// of the last, and locking that takes words nobody picked — measured, a
    /// 39-character selection took 68. So the shapes travel whole, one per
    /// line, and the engine keeps their union only for the badge that undoes
    /// the lock. A dragged rectangle is one shape and takes the same path.
    pub fn lock_shapes(
        &self,
        page_index: usize,
        shapes: &[pdf_core::document::Rect],
        passcode: &[u8],
        outlined_fonts: &[&[u8]],
        require_complete: bool,
    ) -> Result<pdf_core::document::RedactionReport> {
        let catalogue = merged_catalogue(outlined_fonts);
        let request = pdf_core::document::Redaction::over(page_index, shapes.to_vec())
            .ok_or(pdf_core::PdfError::InvalidArgument("nothing to lock".into()))?;
        let request = pdf_core::document::Redaction { require_complete, ..request };
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("locking this document"))?
                .lock_area(&request, passcode, catalogue.as_ref())
        })
    }

    /// Finish any lock that recorded its badge but never took its picture off
    /// the page. Returns how many were completed and how many dropped.
    ///
    /// Nothing is touched on a document with no such lock — see
    /// [`pdf_core::document::DocumentMut::repair_locks`].
    pub fn repair_locks(&self) -> Result<(usize, usize)> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("repairing this document"))?
                .repair_locks()
        })
    }

    /// What a page draws, in the order it draws it — bottom first.
    pub fn drawn_objects(&self, page_index: usize) -> Result<Vec<pdf_core::document::DrawnObject>> {
        registry::with_session(self.handle, |s| s.document.drawn_objects(page_index))
    }

    /// Resize one thing about a point — see
    /// [`pdf_core::document::DocumentMut::scale_object`].
    pub fn scale_object(
        &self,
        page: usize,
        object: usize,
        anchor: pdf_core::document::Point,
        sx: f32,
        sy: f32,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.scale_object(page, object, anchor, sx, sy))
    }

    /// Make one thing more or less see-through — see
    /// [`pdf_core::document::DocumentMut::set_opacity`].
    pub fn set_opacity(&self, page: usize, object: usize, opacity: f32) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.set_opacity(page, object, opacity))
    }

    /// What one step up or down would pass: the nearest thing that overlaps
    /// this one in that direction — see
    /// [`pdf_core::document::Document::stacking_neighbour`].
    pub fn stacking_neighbour(
        &self,
        page_index: usize,
        object: usize,
        up: bool,
    ) -> Result<Option<pdf_core::document::DrawnObject>> {
        registry::with_session(self.handle, |s| s.document.stacking_neighbour(page_index, object, up))
    }

    /// Put one thing at the front or the back of a page's drawing order.
    ///
    /// **The only stacking a PDF has is the order it draws things in**, so this
    /// moves the operators that draw it and carries the state they were drawn
    /// under along with them. See [`pdf_core::document::DocumentMut::restack`]
    /// for what it declines.
    pub fn restack(
        &self,
        page_index: usize,
        object: usize,
        where_to: pdf_core::document::Stacking,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.restack(page_index, object, where_to))
    }

    /// Lock whole pages, sealing each and leaving it blank.
    ///
    /// Takes no candidate fonts, and needs none: nothing here has to identify
    /// what is on the page in order to remove it — see
    /// [`pdf_core::document::DocumentMut::lock_pages`], which is why this works
    /// on outlined and scanned pages that [`Self::lock_area`] must refuse.
    /// Offer fonts for typing characters a document's own fonts cannot spell.
    ///
    /// The reader's own fonts, not this program's: a licensed typeface is
    /// theirs to supply. See [`pdf_core::pdf::embed`].
    pub fn set_typing_fonts(&self, fonts: Vec<Vec<u8>>) -> Result<()> {
        registry::with_session(self.handle, |s| {
            if let Some(doc) = s.document.as_document_mut() {
                doc.set_typing_fonts(fonts);
            }
            Ok(())
        })
    }

    /// The face the last edit fell back to, if it was not the run's own.
    pub fn substituted_face(&self) -> Option<String> {
        registry::with_session(self.handle, |s| Ok(s.document.substituted_face()))
            .ok()
            .flatten()
    }

    /// Rule a line, as a form is filled in by hand.
    pub fn stamp_line(
        &self,
        page: usize,
        from: pdf_core::document::Point,
        to: pdf_core::document::Point,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?
                .stamp_line(page, from, to)
        })
    }

    /// Draw a box around something, as a form is filled in by hand.
    pub fn stamp_box(&self, page: usize, area: pdf_core::document::Rect) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?
                .stamp_box(page, area)
        })
    }

    /// What this document permits, when it is encrypted. `None` when it has no
    /// password — see [`pdf_core::document::Document::permissions`].
    pub fn permissions(&self) -> Option<pdf_core::pdf::encrypt::Permissions> {
        registry::with_session(self.handle, |s| Ok(s.document.permissions())).unwrap_or(None)
    }

    /// Place ink and record that it is a signature rather than a drawing.
    ///
    /// Both halves here rather than at the caller, because the index the mark
    /// goes on is PDFium's and is only knowable in between: the engine appends,
    /// so the new annotation is the last one on the page. Doing it in one place
    /// means there is no window in which a placed signature is not one.
    pub fn place_signature(
        &self,
        page: usize,
        strokes: Vec<Vec<pdf_core::document::Point>>,
        color: pdf_core::document::Color,
        width: f32,
        name: &str,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            let doc = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;
            let index =
                doc.add_annotation(page, &pdf_core::document::Annotation::Ink { strokes, color, width })?;
            doc.mark_as_signature(page, index, name)
        })
    }

    /// The signatures placed on a page, as opposed to any other ink on it.
    pub fn signature_marks(
        &self,
        page: usize,
    ) -> Result<Vec<pdf_core::document::SignatureMark>> {
        registry::with_session(self.handle, |s| s.document.signature_marks(page))
    }

    /// Burn a page's placed signatures into the page. Returns how many.
    pub fn apply_signatures(&self, page: usize) -> Result<usize> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?
                .apply_signatures(page)
        })
    }

    /// Check the signatures this document carries.
    pub fn validate_signatures(&self) -> Result<Vec<pdf_core::pdf::validate::Signature>> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("checking this document"))?
                .validate_signatures()
        })
    }

    /// Ask a time authority to attest that this document exists now.
    pub fn timestamp_document(&self, authority: &str) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("timestamping this document"))?
                .timestamp_document(authority)
        })
    }

    /// Sign this document with a certificate.
    pub fn sign_document(
        &self,
        pkcs12: &[u8],
        password: &str,
        about: &pdf_core::pdf::sign::Reason,
    ) -> Result<String> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("signing this document"))?
                .sign_document(pkcs12, password, about)
        })
    }

    /// How many signatures this document carries.
    pub fn signature_count(&self) -> usize {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().map(|d| d.signature_count()).unwrap_or(0))
        })
        .unwrap_or(0)
    }

    /// Put a tick, a cross or a dot on a page.
    pub fn stamp_mark(
        &self,
        page_index: usize,
        mark: pdf_core::document::FillMark,
        at: pdf_core::document::Point,
        size: f32,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("filling in this document"))?
                .stamp_mark(page_index, mark, at, size)
        })
    }

    /// Mark how far this document may travel.
    pub fn set_sensitivity(
        &self,
        level: pdf_core::document::sensitivity::Sensitivity,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("marking this document"))?
                .set_sensitivity(level)
        })
    }

    /// Take the marking off.
    pub fn clear_sensitivity(&self) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("marking this document"))?
                .clear_sensitivity()
        })
    }

    /// How this document is marked, if at all.
    pub fn sensitivity(&self) -> Option<pdf_core::document::sensitivity::Sensitivity> {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().and_then(|d| d.sensitivity()))
        })
        .ok()
        .flatten()
    }

    /// Paint over an area, covering what is there without removing it.
    pub fn whiteout(
        &self,
        page_index: usize,
        area: pdf_core::document::Rect,
        colour: pdf_core::document::Color,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("painting over this document"))?
                .whiteout(page_index, area, colour)
        })
    }

    /// Things on a page somebody would not want to send out.
    pub fn sensitive_on(&self, page_index: usize) -> Result<Vec<pdf_core::document::SensitiveOnPage>> {
        registry::with_session(self.handle, |s| s.document.sensitive_on(page_index))
    }

    /// What this document carries that is not on its pages.
    pub fn hidden_data(&self) -> Result<pdf_core::pdf::hidden::Hidden> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("surveying this document"))?
                .hidden_data()
        })
    }

    /// Take that data out, and say what went — and what did not.
    pub fn remove_hidden_data(&self) -> Result<pdf_core::pdf::hidden::Sanitised> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("sanitising this document"))?
                .remove_hidden_data()
        })
    }

    /// Put a password on the file, written when it is next saved.
    ///
    /// A different promise from locking: that hides content inside a document
    /// anyone can open, this shuts the whole file to anyone without the
    /// password — in Pagify or in any other reader.
    pub fn secure_document(
        &self,
        user: &[u8],
        owner: Option<&[u8]>,
        permissions: pdf_core::pdf::encrypt::Permissions,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("securing this document"))?
                .secure_document(user, owner, permissions)
        })
    }

    /// Put Pagify's own password on the file — stronger, and readable by
    /// nothing else.
    pub fn secure_document_plus(&self, user: &[u8]) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("securing this document"))?
                .secure_document_plus(user)
        })
    }

    /// Whether the password on this document is Pagify's own.
    pub fn is_secure_plus(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.is_secure_plus()))
        })
        .unwrap_or(false)
    }

    /// Take the password off again, before it has been saved with one.
    pub fn unsecure_document(&self) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("securing this document"))?
                .unsecure_document()
        })
    }

    /// Whether the file this came from had a password when it was opened.
    pub fn had_password_on_open(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.had_password_on_open()))
        })
        .unwrap_or(false)
    }

    /// Whether this is the password the document was opened with.
    pub fn password_matches(&self, typed: &[u8]) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.password_matches(typed)))
        })
        .unwrap_or(false)
    }

    /// Whether the file this document came from already has a password.
    pub fn already_has_password(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.already_has_password()))
        })
        .unwrap_or(false)
    }

    /// Whether a password is waiting to be written on the next save.
    pub fn is_secured(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.is_secured()))
        })
        .unwrap_or(false)
    }

    pub fn lock_pages(&self, pages: &[usize], passcode: &[u8]) -> Result<usize> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("locking this document"))?
                .lock_pages(pages, passcode)
        })
    }

    /// Every image on a page, for offering one as something to lock.
    pub fn images_on(&self, page_index: usize) -> Result<Vec<pdf_core::document::PageImage>> {
        registry::with_session(self.handle, |s| s.document.images_on(page_index))
    }

    /// Take one image off its page and seal it. Returns the id naming the seal.
    pub fn lock_image(&self, page_index: usize, object: usize, passcode: &[u8]) -> Result<String> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("locking this document"))?
                .lock_image(page_index, object, passcode)
        })
    }

    /// Put one sealed object back, leaving every other seal alone.
    pub fn unlock_item(&self, id: &str, passcode: &[u8]) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("unlocking this document"))?
                .unlock_item(id, passcode)
        })
    }

    /// What is sealed on one page, for drawing its badges.
    ///
    /// Reached through `as_document_mut` for the same reason `locked_pages` is:
    /// the vault lives beside the writes, even though asking what is in it does
    /// not change anything.
    pub fn locked_items_on(&self, page_index: usize) -> Vec<pdf_core::document::LockedItem> {
        registry::with_session(self.handle, |s| {
            Ok(s.document
                .as_document_mut()
                .and_then(|d| d.locked_items_on(page_index).ok())
                .unwrap_or_default())
        })
        .unwrap_or_default()
    }

    /// Every locked page's original, decrypted and verified. Writes nothing —
    /// the caller puts them back through the command stack so undo works.
    pub fn open_lock(&self, passcode: &[u8]) -> Result<Vec<(usize, Vec<u8>)>> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("unlocking this document"))?
                .open_lock(passcode)
        })
    }

    /// Which pages have a sealed original in this document.
    pub fn locked_pages(&self) -> Vec<usize> {
        registry::with_session(self.handle, |s| {
            Ok(s.document
                .as_document_mut()
                .and_then(|d| d.locked_pages().ok())
                .unwrap_or_default())
        })
        .unwrap_or_default()
    }

    /// Whether saving must rewrite the file rather than append to it.
    ///
    /// True once anything has been redacted. An incremental save keeps the
    /// original bytes and appends a delta, so the removed words would still be
    /// in the file — see `DocumentMut::must_save_full_copy`.
    pub fn must_save_full_copy(&self) -> bool {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().is_some_and(|d| d.must_save_full_copy()))
        })
        .unwrap_or(false)
    }

    pub fn edit_state(&self) -> Result<pdf_core::engine::EditState> {
        registry::with_session(self.handle, |s| Ok(pdf_core::engine::edit_state(s)))
    }

    /// Pull pages out into a new document on disk.
    pub fn extract_to(&self, pages: &[usize], dest: &Path) -> Result<usize> {
        let mut file = std::fs::File::create(dest)?;
        registry::with_session(self.handle, |s| {
            let mut extracted = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("extracting from this document"))?
                .extract_pages(pages)?;
            let count = extracted.page_count();
            extracted
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("saving the extracted pages"))?
                .save_full_copy(&mut file)?;
            Ok(count)
        })
    }

    /// Every run of text on a page, in the order the file stores them.
    pub fn text_runs(&self, page: usize) -> Result<Vec<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_runs(page))
    }

    /// A page's crop box, in page points with a top-left origin.
    pub fn page_crop(&self, index: usize) -> Result<pdf_core::document::Rect> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading this document's boundaries"))
                .and_then(|d| d.page_crop(index))
        })
    }

    /// Copy pages within this document, inserting the copies at `at`.
    ///
    /// The same machinery as importing from another file, aimed at itself: the
    /// pages are extracted into a document of their own, saved to bytes, and
    /// brought back in through `ImportPages`. There is no cheaper route — a page
    /// is not a value that can be cloned, it is a node in a tree with resources
    /// hanging off it, and PDFium's own copy path is the extract.
    ///
    /// It goes through `execute`, so a duplicate can be undone like any other
    /// edit.
    pub fn duplicate_pages(&self, pages: &[usize], at: usize) -> Result<usize> {
        if pages.is_empty() {
            return Err(pdf_core::PdfError::InvalidArgument("no pages to duplicate".into()));
        }
        let command = registry::with_session(self.handle, |s| {
            let mut slice = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("duplicating in this document"))?
                .extract_pages(pages)?;
            let mut bytes = Vec::new();
            slice
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading those pages"))
                .and_then(|d| d.save_full_copy(&mut bytes).map(|_| ()))?;
            Ok(pdf_core::command::Command::ImportPages { at, pdf: bytes })
        })?;

        self.execute(command).map(|state| state.page_count)
    }

    /// Bring pages in from another file.
    pub fn import_from(&self, source: &Path, pages: &[usize], at: usize) -> Result<usize> {
        let source = Session::open(source)?;
        let command = registry::with_session(source.handle, |src| {
            let mut slice = src
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading from that document"))?
                .extract_pages(pages)?;
            let mut bytes = Vec::new();
            slice
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("reading those pages"))
                .and_then(|d| d.save_full_copy(&mut bytes).map(|_| ()))?;
            Ok(pdf_core::command::Command::ImportPages { at, pdf: bytes })
        })?;

        self.execute(command).map(|state| state.page_count)
    }

    /// Highlight a run of text, as one mark covering however many lines.
    pub fn highlight(
        &self,
        page: usize,
        rects: Vec<pdf_core::document::Rect>,
        colour: pdf_core::document::Color,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Highlight { rects, color: colour },
        })
    }

    /// Anchor a note to a point on the page.
    pub fn note(
        &self,
        page: usize,
        rect: pdf_core::document::Rect,
        contents: String,
        colour: pdf_core::document::Color,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Note { rect, contents, color: colour },
        })
    }

    /// What kind of text, if any, a page has.
    pub fn classify(&self, page: usize) -> Result<pdf_core::document::PageClassification> {
        registry::with_session(self.handle, |s| s.document.page(page)?.classify())
    }

    /// The characters on a page, unpacked for selection.
    pub fn characters(&self, page: usize) -> Result<crate::reader::Characters> {
        registry::with_session(self.handle, |s| {
            let raw = s.document.page(page)?.characters()?;
            Ok(crate::reader::Characters::new(raw))
        })
    }

    /// Every page's size, for laying out the continuous strip.
    pub fn page_sizes(&self) -> Result<Vec<pdf_core::PageSize>> {
        registry::with_session(self.handle, |s| {
            (0..s.document.page_count()).map(|i| s.document.page_size(i)).collect()
        })
    }

    /// Run `f` against the engine session, with the registry lock held.
    ///
    /// The escape hatch for the phases that need more of the engine than this
    /// façade has grown a method for. Note what the lock costs, and that it is
    /// the right cost: a long operation here blocks renders of every other open
    /// document, and the alternative is a page cache that occasionally serves
    /// pages from the wrong file.
    pub fn with_engine<T>(
        &self,
        f: impl FnOnce(&mut pdf_core::registry::DocumentSession) -> Result<T>,
    ) -> Result<T> {
        registry::with_session(self.handle, f)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        registry::remove(self.handle);
    }
}
