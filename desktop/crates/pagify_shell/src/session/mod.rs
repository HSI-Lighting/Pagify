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
mod typing;
mod markup;
mod runs;
mod pages;
mod annotate;
mod save;

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



    // -- editing ------------------------------------------------------------














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
