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

mod locking;
mod signing;
mod objects;
mod io;
mod render;
mod reading;
mod history;
mod security;

use std::path::{Path, PathBuf};

/// Where a write is staged before it replaces its target.
///
/// A sibling, so the rename stays on one filesystem and is therefore atomic.
/// Named so that no two writes — and no leftover from a crash — land on the
/// same path: the old fixed `name.pdf.pagify-save` could be guessed, and a
/// symlink planted there would have been followed. Found by audit. The name
/// is the second guard; the first is that the file is opened with
/// `create_new`, so nothing already at the path is ever written through.
fn staging_path(target: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".pagify-save-{:x}-{:x}-{:x}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    target.with_file_name(name)
}

/// Write a file beside its target and rename it over the target.
///
/// What this guarantees, and every caller relies on:
///
/// - **Nothing at the staging path is ever opened.** `create_new` refuses an
///   existing file, a symlink included, so a path somebody else planted is
///   an error rather than a write through it.
/// - **A target that exists keeps its own permissions.** A document kept at
///   0600 was coming back 0644 after a save, because the staging file had
///   the process's default mode. Found by audit.
/// - **A failure leaves the target exactly as it was**, and nothing beside
///   it: the staging file is removed on any error, before or during the
///   rename.
fn write_then_rename(
    target: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
    write_then_rename_via(target, &staging_path(target), write)
}

/// [`write_then_rename`] with the staging path chosen by the caller — which
/// is only ever a test planting something there.
fn write_then_rename_via(
    target: &Path,
    staging: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
    let outcome = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Owner-only from the moment it exists — the default a brand new
        // destination gets when there is no existing file's mode to inherit
        // below. Without this a "Save As" to a path that never existed
        // landed at `0666 & ~umask` (0644 under a typical umask) even when
        // the document being saved was 0600, because the inheritance a few
        // lines down only ever fires for an *overwrite*. Found by audit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&staging)?;
        write(&mut file)?;
        file.sync_all()?;
        if let Ok(existing) = std::fs::metadata(target) {
            std::fs::set_permissions(&staging, existing.permissions())?;
        }
        Ok(())
    })();
    if let Err(problem) = outcome {
        // Only what this created: a file that was already there is somebody
        // else's, and is exactly what `create_new` refused to touch.
        if !matches!(&problem, pdf_core::PdfError::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists)
        {
            let _ = std::fs::remove_file(staging);
        }
        return Err(problem);
    }
    replace_target(staging, target)
}

/// Whether an error is Windows saying the file is open in another program
/// (ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, or ERROR_ACCESS_DENIED for a
/// file held without delete sharing) — a scanner or indexer that has just
/// looked at the new file, another Pagify window with the same document open,
/// a preview pane. Never true on another platform.
fn is_in_use(e: &std::io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5) | Some(32) | Some(33))
}

/// Swap the finished staging file over the target.
///
/// **Reported from use: "save failed: i/o error: the process cannot access the
/// file because it is being used by another process (os error 32)", and a
/// copy of the document left beside it.** The whole new file was written and
/// only the swap was refused, because something else had one of the two files
/// open. That is usually gone a moment later, so the swap is tried again for
/// about two seconds. When it is not, the finished copy is **kept** under a
/// name a person can use (`<name> (saved copy).pdf`) and the error says where
/// it is — deleting it, which every other failure here does, would throw away
/// the edits the person was saving.
fn replace_target(staging: &Path, target: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..25 {
        match std::fs::rename(staging, target) {
            Ok(()) => return Ok(()),
            Err(e) if is_in_use(&e) => {
                last = Some(e);
                if attempt < 24 {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(staging);
                return Err(pdf_core::PdfError::Io(e));
            }
        }
    }
    let e = last.expect("the loop only ends with an in-use error");
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let kept = (0..100)
        .map(|n| match n {
            0 => dir.join(format!("{stem} (saved copy).pdf")),
            n => dir.join(format!("{stem} (saved copy {}).pdf", n + 1)),
        })
        .find(|p| !p.exists())
        .and_then(|p| std::fs::rename(staging, &p).ok().map(|_| p));
    match kept {
        Some(copy) => Err(pdf_core::PdfError::Io(std::io::Error::new(
            e.kind(),
            format!(
                "{} is open in another program (another Pagify window with the same file?), so it could not \
                 be replaced. Your changes are saved in {} — close the other program, then use that copy.",
                target.display(),
                copy.display()
            ),
        ))),
        None => {
            let _ = std::fs::remove_file(staging);
            Err(pdf_core::PdfError::Io(e))
        }
    }
}

#[cfg(test)]
mod staging_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pagify-staging-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    /// Two saves of one file never stage at the same place, so nothing can be
    /// waiting there.
    #[test]
    fn no_two_staging_paths_are_the_same() {
        let target = Path::new("/tmp/doc.pdf");
        let a = staging_path(target);
        let b = staging_path(target);
        assert_ne!(a, b);
        assert_eq!(a.parent(), target.parent(), "staged somewhere other than beside the target");
    }

    /// **A file already at the staging path is not written through** — not a
    /// plain file, and not a symlink to something else. Found by audit.
    #[test]
    fn something_planted_at_the_staging_path_is_refused_and_left_alone() {
        let dir = scratch("planted");
        let target = dir.join("doc.pdf");
        std::fs::write(&target, b"the document").expect("target");
        let planted = dir.join("doc.pdf.pagify-save-planted");
        std::fs::write(&planted, b"planted").expect("plant");

        let outcome = write_then_rename_via(&target, &planted, |f| {
            use std::io::Write;
            f.write_all(b"new contents").map_err(pdf_core::PdfError::Io)
        });
        assert!(outcome.is_err(), "a planted file was opened for writing");
        assert_eq!(std::fs::read(&planted).expect("read"), b"planted", "the planted file was written through");
        assert_eq!(std::fs::read(&target).expect("read"), b"the document", "the target changed");

        #[cfg(unix)]
        {
            let victim = dir.join("victim");
            std::fs::write(&victim, b"untouched").expect("victim");
            let link = dir.join("doc.pdf.pagify-save-link");
            std::os::unix::fs::symlink(&victim, &link).expect("symlink");
            let outcome = write_then_rename_via(&target, &link, |f| {
                use std::io::Write;
                f.write_all(b"through the link").map_err(pdf_core::PdfError::Io)
            });
            assert!(outcome.is_err(), "a symlink at the staging path was followed");
            assert_eq!(std::fs::read(&victim).expect("read"), b"untouched");
            assert!(link.symlink_metadata().is_ok(), "the symlink was removed as if it were ours");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
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

    /// This raster's own pixels within `(left, top)`–`(right, bottom)`,
    /// clamped to what the raster actually has rather than out of bounds —
    /// narrower, shorter, or empty at an edge or a corner. RGBA, the same
    /// layout as [`Self::pixels`]; the returned `(width, height)` go
    /// alongside it since a clipped rectangle can come back smaller than
    /// asked for, and an empty one comes back as `(Vec::new(), 0, 0)`.
    ///
    /// **What this is for.** A picture composited against this crop —
    /// [`crate::signatures::composite_onto_image`] — looks exactly like the
    /// page underneath it wherever the picture is transparent, because it
    /// *is* the page underneath it, copied pixel for pixel rather than
    /// approximated as one flat colour. The one thing it cannot survive is
    /// the page changing after the crop was taken.
    pub fn crop(&self, left: u32, top: u32, right: u32, bottom: u32) -> (Vec<u8>, u32, u32) {
        let (left, top) = (left.min(self.width), top.min(self.height));
        let (right, bottom) =
            (right.min(self.width.saturating_sub(1)), bottom.min(self.height.saturating_sub(1)));
        if self.width == 0 || self.height == 0 || left > right || top > bottom {
            return (Vec::new(), 0, 0);
        }
        let (w, h) = (right - left + 1, bottom - top + 1);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in top..=bottom {
            let row_start = ((y * self.width + left) * 4) as usize;
            out.extend_from_slice(&self.pixels[row_start..row_start + (w * 4) as usize]);
        }
        (out, w, h)
    }
}

#[cfg(test)]
mod raster_tests {
    use super::PageRaster;

    /// A raster filled with `fill`, except for a `border`-coloured ring
    /// `border`-px wide around the edge — a page with a plain, uniform
    /// background and something else in the middle, the shape every real
    /// page a signature gets sampled against roughly has: blank paper at
    /// the edges of the line it is signing, whatever else nearby.
    fn ringed(width: u32, height: u32, border: [u8; 3], fill: [u8; 3]) -> PageRaster {
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let on_edge = x < 3 || y < 3 || x >= width - 3 || y >= height - 3;
                let c = if on_edge { border } else { fill };
                let i = ((y * width + x) * 4) as usize;
                pixels[i..i + 3].copy_from_slice(&c);
                pixels[i + 3] = 255;
            }
        }
        PageRaster { width, height, pixels, from_cache: false }
    }

    /// **The crop holds the interior's own pixels, not the border's** —
    /// unlike an averaged sample, a crop taken from inside the ring must
    /// come back as the fill colour throughout, never touched by the border
    /// a few pixels further out.
    #[test]
    fn a_crop_holds_the_interior_pixels_exactly() {
        let raster = ringed(100, 100, [10, 20, 30], [200, 200, 200]);
        let (pixels, w, h) = raster.crop(10, 10, 89, 89);
        assert_eq!((w, h), (80, 80));
        assert!(
            pixels.chunks_exact(4).all(|p| p[0..3] == [200, 200, 200]),
            "the crop should be entirely the fill colour, found border pixels leaking in"
        );
    }

    /// A rectangle only partly on the raster comes back narrower or
    /// shorter than asked for, clamped rather than reading out of bounds;
    /// one entirely off the raster, or inverted, comes back empty rather
    /// than panicking.
    #[test]
    fn a_partly_or_wholly_out_of_range_rectangle_is_clamped_not_read_out_of_bounds() {
        let raster = ringed(50, 50, [0, 0, 0], [220, 220, 220]);
        let (pixels, w, h) = raster.crop(40, 40, 100, 100);
        assert_eq!((w, h), (10, 10), "should have clamped to the raster's own edge");
        assert_eq!(pixels.len(), (10 * 10 * 4) as usize);

        let (pixels, w, h) = raster.crop(10, 10, 5, 5);
        assert_eq!((w, h, pixels.len()), (0, 0, 0), "an inverted rectangle should come back empty");

        let (pixels, w, h) =
            PageRaster { width: 0, height: 0, pixels: Vec::new(), from_cache: false }.crop(0, 0, 10, 10);
        assert_eq!((w, h, pixels.len()), (0, 0, 0), "an empty raster should come back empty");
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

/// Everything the paragraph detector reads about one page's text, taken in one
/// pass under one registry lock — see [`Session::page_text_snapshot`].
///
/// A copy: it stays true to the page as it was when taken, and is for the
/// caller to keep or drop.
#[derive(Clone, Debug)]
pub struct PageTextSnapshot {
    /// Every text object on the page, in the order the file stores them, with
    /// **nothing filtered out**: a blank run, a run with no ink area and a run
    /// drawn at alpha 0 are all here, for the caller to decide about. `size` is
    /// the effective (scaled) font size.
    pub runs: Vec<pdf_core::document::TextRun>,
    /// Each run's font identity and weight, by object index — see
    /// [`pdf_core::document::RunStyle`].
    pub styles: std::collections::HashMap<usize, pdf_core::document::RunStyle>,
    /// Each run's font name, by object index — what
    /// [`Session::run_font_names`] answers.
    pub faces: std::collections::HashMap<usize, String>,
    /// The page's drawn paths (rules, boxes and words converted to outlines),
    /// bottom first, as [`Session::drawn_objects`] reports them. **Empty when
    /// the page has no text objects at all**, without having been asked for:
    /// a page of only outlines has tens of thousands of them, and nothing here
    /// has a use for shapes on a page with no text to bridge between.
    pub shapes: Vec<pdf_core::document::DrawnObject>,
}

/// An open document. Owns its registry entry for as long as it lives.
pub struct Session {
    handle: i64,
    path: PathBuf,
    /// The undo history's change counter — the history's own, shared out, so it
    /// can be read without the registry lock. See [`Session::undo_generation`].
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Session {


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
        write_then_rename(path, |file| {
            registry::with_session(self.handle, |s| pdf_core::engine::save(s, file, incremental))
        })
    }

    // -- editing ------------------------------------------------------------


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

    /// Offer one more font for typing — a font picked at the moment of
    /// editing, kept alongside whatever [`Self::set_typing_fonts`] already
    /// holds rather than replacing it. See
    /// [`pdf_core::document::DocumentMut::add_typing_font`].
    pub fn add_typing_font(&self, font: Vec<u8>) -> Result<()> {
        registry::with_session(self.handle, |s| {
            if let Some(doc) = s.document.as_document_mut() {
                doc.add_typing_font(font);
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

    /// Whether the file had to be mended in memory to be opened — see
    /// [`pdf_core::document::Document::repaired_on_open`].
    pub fn repaired_on_open(&self) -> bool {
        registry::with_session(self.handle, |s| Ok(s.document.repaired_on_open())).unwrap_or(false)
    }


    /// How many signatures this document carries.
    pub fn signature_count(&self) -> usize {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().map(|d| d.signature_count()).unwrap_or(0))
        })
        .unwrap_or(0)
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
    ///
    /// Staged and renamed like a save: the destination used to be created —
    /// truncated — before the pages were checked, so a bad page number left
    /// an empty file where a document may have been. Found by audit.
    pub fn extract_to(&self, pages: &[usize], dest: &Path) -> Result<usize> {
        let mut count = 0;
        write_then_rename(dest, |file| {
            registry::with_session(self.handle, |s| {
                let mut extracted = s
                    .document
                    .as_document_mut()
                    .ok_or(pdf_core::PdfError::Unsupported("extracting from this document"))?
                    .extract_pages(pages)?;
                count = extracted.page_count();
                extracted
                    .as_document_mut()
                    .ok_or(pdf_core::PdfError::Unsupported("saving the extracted pages"))?
                    .save_full_copy(file)
            })
        })?;
        Ok(count)
    }

    /// Every run of text on a page, in the order the file stores them.
    pub fn text_runs(&self, page: usize) -> Result<Vec<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_runs(page))
    }

    /// Every text object's bounding rect, without its words — see
    /// [`pdf_core::document::Document::text_run_rects`].
    pub fn text_run_rects(&self, page: usize) -> Result<Vec<(usize, pdf_core::document::Rect)>> {
        registry::with_session(self.handle, |s| s.document.text_run_rects(page))
    }

    /// One run, by object number, words included — see
    /// [`pdf_core::document::Document::text_run_at`].
    pub fn text_run_at(
        &self,
        page: usize,
        object: usize,
    ) -> Result<Option<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_run_at(page, object))
    }

    /// Full runs, words included, for just the given objects — see
    /// [`pdf_core::document::Document::text_runs_some`].
    pub fn text_runs_some(
        &self,
        page: usize,
        wanted: &std::collections::HashSet<usize>,
    ) -> Result<Vec<pdf_core::document::TextRun>> {
        registry::with_session(self.handle, |s| s.document.text_runs_some(page, wanted))
    }

    /// Everything the paragraph detector reads about one page's text, in one
    /// pass: every text object (nothing filtered), its font identity and
    /// weight, its font name, and the page's drawn shapes.
    ///
    /// **One registry lock for all four reads**, not four. Each of them walks
    /// the page, and taking the lock separately for each would let another
    /// thread's open, render or edit slip in between them — a run list from
    /// before an edit beside a shape list from after it. See
    /// [`PageTextSnapshot`] for what each part holds, and
    /// [`pdf_core::document::Document::text_runs_unfiltered`] for why the runs
    /// are not `text_runs`' (which drops those with no ink area).
    ///
    /// The shapes are not read at all for a page with no text objects: a page
    /// of only outlines has tens of thousands of them, and the walk that lists
    /// them is the slowest of the four.
    pub fn page_text_snapshot(&self, page: usize) -> Result<PageTextSnapshot> {
        registry::with_session(self.handle, |s| {
            let runs = s.document.text_runs_unfiltered(page)?;
            let styles = s.document.run_styles(page)?;
            let faces = s.document.run_font_names(page)?;
            let shapes = if runs.is_empty() { Vec::new() } else { s.document.drawn_shapes(page)? };
            Ok(PageTextSnapshot { runs, styles, faces, shapes })
        })
    }

    /// How many objects a page holds and how many of them are text, **counted
    /// without reading any of it** — see [`pdf_core::document::PageScale`].
    ///
    /// For deciding whether to call [`Session::page_text_snapshot`] at all: that
    /// reads every text object's words and lists every path, and what it costs
    /// grows with these numbers — linearly now (a page of 40,000 words in about
    /// 0.4 s; it took five minutes while the words were read one object at a
    /// time), and a drawing of 880,000 paths is still seconds. This is a page
    /// open and one question per object: milliseconds on a datasheet page, a
    /// second on a drawing of 880,000 paths (PDFium parses the whole page to open
    /// it — no count can be had without that).
    pub fn page_scale(&self, page: usize) -> Result<pdf_core::document::PageScale> {
        registry::with_session(self.handle, |s| s.document.page_scale(page))
    }

    /// **Temporary diagnostic** — see
    /// [`pdf_core::document::Document::take_last_batch_timing`].
    pub fn take_last_batch_timing(&self) -> Vec<(&'static str, std::time::Duration)> {
        registry::with_session(self.handle, |s| Ok(s.document.take_last_batch_timing()))
            .unwrap_or_default()
    }

    /// One object per character, in place of a run of them — see
    /// [`pdf_core::document::Document::split_run_into_characters`].
    pub fn split_run_into_characters(&self, page: usize, object: usize) -> Result<()> {
        registry::with_session(self.handle, |s| s.document.split_run_into_characters(page, object))
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

    /// Make one rectangle of the page a clickable link to `uri`.
    ///
    /// One call per line — see [`pdf_core::document::Annotation::Link`]'s own
    /// doc for why a link over wrapped text is several of these rather than
    /// one annotation with several rects.
    pub fn add_link(
        &self,
        page: usize,
        rect: pdf_core::document::Rect,
        uri: String,
    ) -> Result<pdf_core::engine::EditState> {
        self.execute(pdf_core::command::Command::AddAnnotation {
            page_index: page,
            annotation: pdf_core::document::Annotation::Link { rect, uri },
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

    /// Every bookmark in the document's own outline, title and the page it
    /// goes to, top level only — see [`pdf_core::document::Document::
    /// bookmarks`]'s own doc for why nesting is not modelled.
    pub fn bookmarks(&self) -> Result<Vec<(String, usize)>> {
        registry::with_session(self.handle, |s| s.document.bookmarks())
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
