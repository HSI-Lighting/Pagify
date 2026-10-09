//! Read-only [`Document`] backed by PDFium — the phase 1 implementation.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::raw::{c_char, c_int, c_uint, c_ulong};
use std::sync::OnceLock;

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPagePaperSize,
    PdfPageObjectCommon, PdfPageObjectType, PdfPageObjectsCommon, PdfPageRenderRotation, PdfPoints,
    PdfRenderConfig, Pdfium, PdfiumError, PdfiumLibraryBindingsAccessor,
    FPDFANNOT_COLORTYPE, FPDF_ANNOTATION, FPDF_ANNOTATION_SUBTYPE, FPDF_DOCUMENT, FPDF_FILEWRITE,
    FPDF_FONT, FPDF_PAGE, FPDF_PAGEOBJECT, FPDF_PAGEOBJECTMARK, FS_MATRIX, FS_POINTF,
    FS_QUADPOINTSF, FS_RECTF,
};
use pdfium_render::prelude::PdfiumLibraryBindings;
use pdfium_render::prelude::FPDF_WIDESTRING;
use pdfium_render::prelude::{
    FPDF_PAGEOBJ_FORM, FPDF_PAGEOBJ_IMAGE, FPDF_PAGEOBJ_PATH, FPDF_PAGEOBJ_TEXT,
};

use crate::document::metadata::DocumentMetadata;
use crate::document::{
    Annotation, Color, Document, DocumentMut, Glyph, IndexedAnnotation, InternalLink, Page, PageCharacters,
    PageClassification, PageSize, PageTextKind, RecognisedWord, TEXT_LAYER_ID,
    Point, Rect, RegionRequest, RemovedPage, RenderRequest, Rotation, Ruling, TextSegment,
    Redaction, RedactionReport, Uncleared,
};
use crate::crypto::vault::{self, Vault};
use crate::crypto::{KdfParams, Secret};
use crate::error::{classify_pdfium_load_error, PdfError, Result};
use crate::render::bitmap::{self, Bitmap, PixelOrder};
use crate::render::{RegionPixels, RenderTarget};

/// The process-wide PDFium binding.
///
/// Leaked deliberately. PDFium's own `FPDF_InitLibrary`/`DestroyLibrary` pair is
/// process-global anyway, and a `&'static Pdfium` is what lets an open
/// `PdfDocument<'static>` live in the handle registry without a self-referential
/// struct. The leak is one allocation for the lifetime of the process.
static PDFIUM: OnceLock<std::result::Result<&'static Pdfium, String>> = OnceLock::new();

/// An explicit PDFium location, set before the first open.
///
/// iOS has no system PDFium to find by soname and, at the pinned chromium/7881
/// tag, no static archive published either — so the app embeds
/// `libpdfium.dylib` and passes the bundle path, which it only learns at
/// runtime. A `OnceLock` rather than an env var because the value is read from
/// several threads and `set_var` is not sound alongside them.
static LIBRARY_PATH: OnceLock<String> = OnceLock::new();

/// Point the engine at a PDFium build. Returns `false` if a path was already set
/// or PDFium is already bound, in which case this call changed nothing.
pub fn set_library_path(path: String) -> bool {
    LIBRARY_PATH.set(path).is_ok() && PDFIUM.get().is_none()
}

/// `pdfium-render`'s own `LoadLibraryError` message names the library by its
/// Unix soname (`libpdfium.so`) unconditionally — a generic, cross-platform
/// placeholder, not the path this process actually tried. On Windows this
/// read as "looking for the wrong file entirely", which is not what
/// happened and sent a report chasing the wrong thing.
///
/// **Reported from use, on a fresh install**: `pdfium.dll` was sitting right
/// there next to `Pagify.exe` — confirmed in Explorer — and Windows still
/// refused to load it (`LoadLibraryExW` error 126, "module not found", which
/// despite its name is also what a dependency failure or an outright refusal
/// to load an unrecognised file reports). "The file is missing" and "the
/// file is there but got refused" want different next steps, so this checks
/// which one actually happened before saying anything.
fn describe_bind_failure(path: &str, error: &PdfiumError) -> String {
    if !std::path::Path::new(path).is_file() {
        return format!("could not find {path} — the install may be incomplete.");
    }
    let unblock_hint = if cfg!(target_os = "windows") {
        " On Windows, this usually means the file is missing a dependency, or \
         Windows is refusing to load an unrecognised, unsigned file — right-click \
         it, choose Properties, and look for an \"Unblock\" checkbox at the \
         bottom of the General tab."
    } else {
        ""
    };
    format!(
        "{path} is there, but the system would not load it.{unblock_hint} \
         (reported as: {error})"
    )
}

#[cfg(test)]
mod describe_bind_failure_tests {
    use super::*;

    #[test]
    fn a_missing_path_says_so_without_touching_the_raw_error() {
        let message = describe_bind_failure(
            "Z:\\nowhere\\pdfium.dll",
            &PdfiumError::UnrecognizedPath,
        );
        assert!(message.contains("could not find"));
        assert!(message.contains("Z:\\nowhere\\pdfium.dll"));
        assert!(!message.contains("libpdfium.so"), "must not repeat the misleading soname");
    }

    #[test]
    fn an_existing_path_that_failed_to_load_names_itself_not_the_soname() {
        let dir = std::env::temp_dir().join(format!("pdfium-bind-failure-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("pdfium.dll");
        std::fs::write(&path, b"not a real library").expect("write stand-in file");

        let message = describe_bind_failure(path.to_str().unwrap(), &PdfiumError::UnrecognizedPath);
        assert!(message.contains("is there, but the system would not load it"));
        assert!(
            !message.contains("libpdfium.so"),
            "must describe the real path, not pdfium-render's own generic soname: {message}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

pub fn pdfium() -> Result<&'static Pdfium> {
    PDFIUM
        .get_or_init(|| {
            // On Android `libpdfium.so` is packaged into the APK next to
            // `libpdf_core.so`, so the system loader finds it by soname with no
            // path. A desktop host has no such library to find, which is what
            // `PAGIFY_PDFIUM_LIB` is for: it points at a desktop build of the
            // *pinned* PDFium, and it is what lets the round-trip and corpus
            // tests — every acceptance criterion in phases A and B — run
            // off-device at all. Unset in the app, so the Android path is
            // untouched.
            let bindings = match (LIBRARY_PATH.get(), std::env::var("PAGIFY_PDFIUM_LIB")) {
                // Set by the app (iOS), and first because it is the deliberate
                // one: an env var left over in a test runner must not win over a
                // path the running app chose.
                (Some(path), _) => {
                    Pdfium::bind_to_library(path).map_err(|e| describe_bind_failure(path, &e))?
                }
                (None, Ok(path)) => {
                    Pdfium::bind_to_library(&path).map_err(|e| describe_bind_failure(&path, &e))?
                }
                (None, Err(_)) => Pdfium::bind_to_system_library().map_err(|e| e.to_string())?,
            };
            Ok(&*Box::leak(Box::new(Pdfium::new(bindings))))
        })
        .as_ref()
        .copied()
        .map_err(|e| PdfError::LibraryUnavailable(e.clone()))
}

/// Where an open document's bytes came from. Held only to keep in-memory sources
/// alive and to let the UI show a source description; PDFium owns its own reader.
#[derive(Debug, Clone)]
pub enum DocumentSource {
    Path(String),
    /// A file descriptor handed over by the Storage Access Framework.
    FileDescriptor(i32),
    Memory {
        byte_len: usize,
    },
}

/// A password waiting to be written onto the file.
#[derive(Debug, Clone)]
///
/// **Wiped when dropped.** A password waiting to be written is held here
/// until the save; `Zeroizing` sees to it that when the document goes, the
/// bytes go too, rather than staying in freed memory for as long as the
/// allocator leaves them. Found by audit: the derived keys were already
/// wiped, and the passwords they came from were not.
struct Wanted {
    user: zeroize::Zeroizing<Vec<u8>>,
    owner: Option<zeroize::Zeroizing<Vec<u8>>>,
    permissions: crate::pdf::encrypt::Permissions,
}

/// What the last read of the lock attachment found: the lock, no lock, or
/// why it could not be read.
type CachedVault = std::result::Result<Option<Vault>, String>;

pub struct PdfiumDocument {
    document: PdfDocument<'static>,
    source: DocumentSource,
    page_count: usize,
    /// Set by any mutation, so a caller can ask whether a save is owed.
    dirty: bool,
    /// The lock as last read, so it is not pulled out of the file again for
    /// every question asked about it.
    ///
    /// **What made a locked document unusable.** `locked_items_on` is called
    /// while drawing — once per visible page, every frame — and went through
    /// `read_vault`, which copies the attachment out of the file, parses it as
    /// JSON and base64-decodes every sealed page in it. Measured at **57 ms a
    /// call** with eight pages locked, against a frame budget of sixteen; with
    /// a whole document locked the vault holds the whole document.
    ///
    /// A `Mutex` rather than a `RefCell` because this type is `Sync`, and the
    /// outer `None` means "not read yet" against an inner `None` meaning "read,
    /// and there is no lock".
    ///
    /// **A failed read is cached too.** The lock is read while drawing, and
    /// an attachment named as ours that does not parse used to fall through
    /// `?` before the cache was written — so a large, bad `pagify-lock.json`
    /// was pulled out of the file and re-parsed every frame. Found by audit.
    /// The failure is kept, as its message, until something rewrites the
    /// attachment or reopens the document.
    vault: std::sync::Mutex<Option<CachedVault>>,
    /// Fonts a caller has offered for typing characters the document's own
    /// fonts cannot spell.
    ///
    /// **Held rather than bundled** because whose font it is matters: a
    /// licensed typeface is the reader's to supply, not this crate's to ship.
    /// Empty by default, and an empty list simply means editing refuses where
    /// it refused before.
    typing_fonts: Vec<Vec<u8>>,
    /// The face the last edit had to fall back to, when it was not the run's
    /// own. `None` when the words were written in the font that was there.
    substituted: Option<String>,
    /// The bytes this document is byte-for-byte a copy of, when they are known.
    ///
    /// **Why a copy is worth its memory.** A signature covers an exact stretch
    /// of an exact file. Asking PDFium to write the document out again gives a
    /// *different* file — same pages, different layout — and the signature's
    /// byte range then points into the wrong place. Measured on a page signed
    /// in this process: the range covered 1458 bytes of a 17826-byte re-save,
    /// so a correctly signed document was reported as only partly covered.
    ///
    /// So the bytes are kept at the moment they are produced — signing has
    /// them in hand — rather than reconstructed later from something that
    /// cannot reproduce them. A document opened from a
    /// path needs nothing here: its file is on disk and can be read back.
    /// `None` means "ask the source", and it is the ordinary case.
    written: Option<Vec<u8>>,
    /// Whether `written` holds bytes that have not reached the disk.
    ///
    /// **Signing produces the file, and so does an edit appended to a signed
    /// one; nothing else can.** The signed bytes are exact — a byte range and
    /// a digest over it — and the only way they get to disk is to be written
    /// verbatim. A save that asked PDFium to write
    /// the document again relocated every object and broke the signature it
    /// had just made; a close discarded it without a word, because the
    /// document had been marked clean to stop that save. Found by audit: a
    /// certified document could not actually be saved. Set by signing,
    /// cleared by the save that writes the bytes, and by any edit — after
    /// which the file to write is those bytes plus a revision.
    exact_pending: bool,
    /// Whether `written`'s **page content** — as opposed to its annotations —
    /// still matches what PDFium currently holds.
    ///
    /// A narrower question than `dirty`, and found missing when placing a
    /// signature after signing, then applying it, still rewrote the whole
    /// file: `add_annotation` calls the general `touch()`, which clears
    /// `exact_pending`, and a placed-but-not-yet-applied annotation has
    /// nothing else to re-set it — so the very same signing-then-signature
    /// flow this whole mechanism exists for defeated it. An annotation lives
    /// in `/Annots`, not in a content stream; it makes the document `dirty`
    /// (a save is owed) without making its content stale (nothing an
    /// appended edit reads from `written` has changed). `touch_annotation`
    /// is the lighter call that keeps the two apart.
    exact_content: bool,
    /// Whether the file had to be mended to be opened at all — see
    /// [`repair_misplaced_xref_type`]. The document in memory is sound; the file
    /// it came from is not, until this is saved over it.
    repaired_on_open: bool,
    /// Whether this document was opened by giving a password.
    ///
    /// Which is to say: the file it came from is already encrypted, and cannot
    /// be given a second password without the first coming off. Recorded on
    /// opening because finding out later means writing the whole document out
    /// and parsing it again — 80 MB, to answer a question the open already
    /// answered.
    already_secured: bool,
    /// Whether the password already on the file is to come off when it is next
    /// saved.
    ///
    /// PDFium keeps a document's encryption when it saves one it opened
    /// encrypted, which is right by default and wrong here — measured, a
    /// "plain copy" saved this way still wanted the password. Removing it takes
    /// a save flag, and the result is checked rather than trusted.
    remove_password: bool,
    /// The password this document was opened with.
    ///
    /// Kept so that changing a password can ask for the current one and check
    /// it. **Not an additional exposure worth worrying about**: the decrypted
    /// document is already in this process's memory, and anyone who could read
    /// this could read that. It is dropped with the document.
    opened_with: Option<zeroize::Zeroizing<Vec<u8>>>,
    /// A password to put on the file when it is next saved.
    ///
    /// The password is kept rather than the derived key, because every save
    /// draws fresh salts and a fresh file key — saving the same document twice
    /// should not produce the same ciphertext twice.
    security: Option<Wanted>,
    /// Whether the password on this document is Pagify's own rather than PDF's.
    ///
    /// Decides which handler writes it back, and decides what the person is
    /// warned about: a Secure Plus document cannot be opened by any other
    /// reader, and that has to be said before it is chosen rather than
    /// discovered afterwards.
    secure_plus: bool,
    /// Set once anything has been redacted, and never cleared.
    ///
    /// **What makes redaction real.** `FPDF_INCREMENTAL` leaves the original
    /// bytes verbatim and appends a delta, so the removed words stay in the
    /// file at their old offsets and any reader that walks the earlier
    /// cross-reference section finds them. Once this is set, saving must
    /// rewrite the whole file.
    redacted: bool,
    /// A placed picture signature's original, alpha-bearing pixels, kept
    /// outside the annotation PDFium actually holds — keyed by an id written
    /// onto the annotation alongside [`SIGNATURE_KEY`].
    ///
    /// **Why this exists at all.** The object a picture is placed through —
    /// `FPDFImageObj_SetBitmap` — drops alpha even in memory, proved by a
    /// dedicated probe; what a caller reads back through PDFium's own
    /// annotation API is never better than what went in. So a picture with
    /// real alpha (from [`crate::document::Annotation::Image`] as produced by
    /// signature extraction) would look exactly as opaque once burned into
    /// the page as it does while merely placed, unless the *original* pixels
    /// are kept somewhere PDFium's round trip cannot touch. This is that
    /// somewhere: `apply_signatures` reads from here first and only falls
    /// back to the (opaque, background-matched) annotation pixels when an id
    /// is not found.
    ///
    /// **Deliberately not written into the file — same-session only.** A
    /// signature placed, saved, closed and reopened before being applied
    /// loses its entry here; applying it then falls back to the
    /// already-opaque annotation pixels rather than failing, so the gap is a
    /// quieter picture, never a missing one. Persisting the original pixels
    /// across a save is a larger, separate piece of work — this is the
    /// scope actually asked for.
    image_alpha: std::collections::HashMap<u64, Vec<u8>>,
    /// The next id [`Self::image_alpha`] hands out. Monotonic for the life of
    /// this document — ids are never reused, so a stale one (an annotation
    /// whose entry was already consumed by `apply_signatures`, or dropped
    /// with a closed document) can never collide with a live one.
    next_alpha_id: u64,
    /// **Temporary.** Where [`Self::set_runs_in_stream`] last spent its own
    /// time — overwritten on every call, read back by
    /// [`Self::last_batch_timing`] right after, for a caller to put in its
    /// own log. Reported from use: a paragraph apply that measured fast,
    /// repeatedly, as an isolated single edit in a fresh test still took
    /// several seconds in the actual running app, growing across a session
    /// in a way a fresh-document-per-test could not reproduce — this exists
    /// to see, from the real session itself, which part of the real call
    /// actually spent that time, since the usual `eprintln!` this file would
    /// otherwise reach for has nowhere to go in a windowed build with no
    /// console attached.
    last_batch_timing: Vec<(&'static str, std::time::Duration)>,
}

/// What an edit starts from: some bytes, and whether they are the document's
/// file exactly as it exists — in which case the edit is appended to them.
struct EditBase {
    bytes: Vec<u8>,
    exact: bool,
}

// `PdfDocument` already carries these under pdfium-render's `thread_safe` feature,
// which routes every PDFium call through a global mutex. The one piece of
// interior mutability here — the cached vault — is behind a `Mutex` of its own,
// so the guarantee is inherited unchanged.
unsafe impl Send for PdfiumDocument {}
unsafe impl Sync for PdfiumDocument {}

impl PdfiumDocument {
    /// Write a signed document's bytes verbatim, when that is what the
    /// document is. `Ok(true)` when it was written; the caller has nothing
    /// more to write.
    ///
    /// Two cases, one rule — **a signed document that has not changed is its
    /// bytes**: the ones just produced by signing and not yet on disk, and
    /// the ones on disk when nothing has been done since. Re-serialising
    /// either through PDFium relocates every object and breaks the signature
    /// for nothing.
    fn write_exact_if_unchanged(&mut self, dest: &mut dyn Write) -> Result<bool> {
        let unchanged = !self.dirty
            && !self.remove_password
            && self.security.is_none()
            && !self.redacted
            && PdfiumDocument::signature_count(self) > 0;
        if !(self.exact_pending || unchanged) {
            return Ok(false);
        }
        let bytes = match (&self.written, &self.source) {
            (Some(exact), _) => exact.clone(),
            (None, DocumentSource::Path(path)) if unchanged => std::fs::read(path)?,
            _ => {
                // Pending with nothing kept cannot happen; treated as nothing
                // pending rather than as a reason to write nothing.
                self.exact_pending = false;
                return Ok(false);
            }
        };
        dest.write_all(&bytes)?;
        self.dirty = false;
        self.exact_pending = false;
        Ok(true)
    }

    /// Record a change: a save is owed, and the file to write is no longer
    /// exactly the one a signature was made over, if one was.
    fn touch(&mut self) {
        self.dirty = true;
        self.exact_pending = false;
        self.exact_content = false;
    }

    /// Record a change to a page's annotations only — a save is owed, but
    /// nothing an appended edit starts from has changed. See `exact_content`.
    fn touch_annotation(&mut self) {
        self.dirty = true;
    }

    /// The bytes an edit through Pagify's own writer starts from.
    ///
    /// For a signed document that is its file exactly — see
    /// [`Self::exact_bytes`] — so that the edit can be *appended* and the
    /// signed bytes stay where the signature's range says they are. For
    /// anything else, PDFium's re-serialisation, which the writer then
    /// rewrites: the same as ever.
    fn edit_base(&self) -> Result<EditBase> {
        if let Some(bytes) = self.exact_bytes() {
            return Ok(EditBase { bytes, exact: true });
        }
        Ok(EditBase { bytes: self.readable_bytes()?, exact: false })
    }

    /// The document's file byte for byte, when it is signed and nothing is
    /// pending inside PDFium that those bytes would not carry.
    ///
    /// A signature is a byte range, and an edit that starts from PDFium's
    /// own re-serialisation moves every object before the edit is even made;
    /// found when the check stopped taking "the range does not reach the end"
    /// as proof of an appended revision and looked at the digest — a whiteout
    /// after signing had been breaking the signature outright while being
    /// reported as a later revision. Only a signed document gets this: for
    /// any other the earlier revision an append leaves behind is a cost with
    /// nothing bought.
    fn exact_bytes(&self) -> Option<Vec<u8>> {
        if self.already_secured || self.security.is_some() || self.remove_password || self.redacted {
            return None;
        }
        if PdfiumDocument::signature_count(self) == 0 {
            return None;
        }
        match (&self.written, &self.source) {
            // Signed or appended to in this process: the bytes kept then, as
            // long as the page content inside PDFium has not changed since —
            // an unapplied annotation does not count, see `exact_content`.
            (Some(exact), _) if self.exact_content || self.exact_pending => Some(exact.clone()),
            (Some(_), _) => None,
            // Opened from disk and untouched: the file itself.
            (None, DocumentSource::Path(path)) if self.exact_content => std::fs::read(path).ok(),
            _ => None,
        }
    }

    /// Write an edit out: appended to the file when the base was the file
    /// exactly, rewritten otherwise.
    fn write_edit(
        base: &EditBase,
        file: &crate::pdf::File<'_>,
        replacements: &[(u32, Vec<u8>)],
        extra: &[(u32, Vec<u8>)],
    ) -> Result<Vec<u8>> {
        if base.exact {
            file.append_revision(replacements, extra)
        } else {
            file.rewrite_adding(replacements, extra, &crate::pdf::Dict(Vec::new()))
        }
    }

    /// Take an edited file on as the document: reopen it, put the security
    /// back, mark the document changed — and, when the edit was appended to
    /// the file exactly, keep the result as the file exactly, so that the
    /// save writes these bytes verbatim and the next edit appends again.
    fn adopt_edit(
        &mut self,
        base: &EditBase,
        rewritten: Vec<u8>,
        was_secured: bool,
        plus: bool,
        permissions: Option<crate::pdf::encrypt::Permissions>,
    ) -> Result<()> {
        let exact = base.exact.then(|| rewritten.clone());
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        self.rearm_security(was_secured, plus, permissions);
        self.touch();
        if let Some(exact) = exact {
            self.written = Some(exact);
            self.exact_pending = true;
        }
        Ok(())
    }

    /// Earlier revisions and orphaned objects exist only in the file as
    /// written; the bytes PDFium re-emits have neither, so a survey of those
    /// alone always reported one revision and nothing unreachable — for a
    /// catalogue with two saves and eight orphans in it. Counted from the
    /// file itself, where there is one.
    fn count_what_only_the_file_shows(&self, found: &mut crate::pdf::hidden::Hidden) {
        let on_disk = match (&self.written, &self.source) {
            (Some(exact), _) => Some(exact.clone()),
            (None, DocumentSource::Path(path)) => std::fs::read(path).ok(),
            (None, _) => None,
        };
        let Some(on_disk) = on_disk else { return };
        let Ok(file) = crate::pdf::File::parse(&on_disk) else { return };
        let Ok(as_written) = crate::pdf::hidden::survey(&file, &on_disk) else { return };
        found.revisions = found.revisions.max(as_written.revisions);
        found.unreachable = found.unreachable.max(as_written.unreachable);
    }

    /// How many signatures a reader finds in this document.
    ///
    /// PDFium's own count, asked so a test can check that what was written is
    /// what somebody else's code sees — rather than only what ours does.
    pub fn signature_count(&self) -> i32 {
        let Ok(pdfium) = pdfium() else { return -1 };
        unsafe { pdfium.bindings().FPDF_GetSignatureCount(self.document.handle()) }
    }

    pub fn open_path(path: &str, password: Option<&str>) -> Result<Self> {
        // Look for our own handler first. A Secure Plus document has to be
        // unsealed here, because PDFium cannot read it — no reader can, which
        // is the point of it.
        //
        // **The look is at the tail of the file, not the whole of it.** Every
        // open used to read the entire document into memory — 80 MB for a
        // catalogue — to search for seven bytes, before PDFium then streamed
        // it. Found by audit. The `/Encrypt` dictionary this writer adds goes
        // after every other object, just before the cross-reference table, so
        // the last stretch of the file is where it is; only a file that shows
        // the mark there is read whole.
        if Self::tail_says_ours(path) {
            let bytes = std::fs::read(path)?;
            if let Some(unsealed) = Self::unseal_if_ours(&bytes, password)? {
                let mut doc = Self::open_bytes(unsealed, None)?;
                doc.source = DocumentSource::Path(path.to_string());
                doc.already_secured = true;
                doc.secure_plus = true;
                doc.opened_with = password.map(|p| zeroize::Zeroizing::new(p.as_bytes().to_vec()));
                return Ok(doc);
            }
        }
        // **A file an earlier build damaged when it saved it.** Reported from use
        // twice: "pdfium error: PdfiumLibraryInternalError(Unknown)" on a
        // document that was fine until Pagify saved it. The fix for what did the
        // damage only stops new files being damaged; the ones already saved stay
        // as they are. Mended in memory here — the file on disk is not touched,
        // and a save writes a sound one.
        //
        // **Looked for before the open, not after it fails.** PDFium opens such a
        // file — it reports the right number of pages — and fails when a page
        // is loaded (poppler: "Kid object (page 1) is wrong type"), which is
        // later, somewhere else, and with a message that says nothing. So the end
        // of the file is asked first, for a damage that leaves a signature.
        if let Some(mended) = Self::mended_if_damaged(path)? {
            let byte_len = mended.len();
            let mut doc = Self::from_reader(
                std::io::Cursor::new(mended),
                password,
                DocumentSource::Memory { byte_len },
            )?;
            doc.repaired_on_open = true;
            return Ok(doc);
        }
        let file = File::open(path)?;
        Self::from_reader(file, password, DocumentSource::Path(path.to_string()))
    }

    /// The file's bytes with the old repair's damage taken out — or `None`, which
    /// is nearly always.
    ///
    /// Only the last stretch of the file is read to ask: the damaged
    /// cross-reference stream is the last object in it. The whole file is read
    /// only for one that is found damaged.
    fn mended_if_damaged(path: &str) -> Result<Option<Vec<u8>>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        let start = length.saturating_sub(Self::TAIL_PROBE);
        file.seek(SeekFrom::Start(start))?;
        let mut tail = Vec::with_capacity((length - start) as usize);
        file.take(Self::TAIL_PROBE).read_to_end(&mut tail)?;
        let Some((open, stray)) = locate_misplaced_xref_type(&tail, start as usize) else {
            return Ok(None);
        };
        let mut bytes = std::fs::read(path)?;
        let (open, stray) = (open + start as usize, stray + start as usize);
        // The same bytes the tail held; if the file moved underfoot, leave it be.
        if bytes.len() != length as usize || bytes.get(stray..stray + MISPLACED_TYPE.len()) != Some(MISPLACED_TYPE) {
            return Ok(None);
        }
        move_xref_type(&mut bytes, open, stray);
        Ok(Some(bytes))
    }

    /// Whether the end of the file carries the Secure Plus handler's name.
    ///
    /// Bounded: the last [`Self::TAIL_PROBE`] bytes, which holds the
    /// dictionary this writer puts there with room to spare. A file that
    /// cannot be read at all is left to `from_reader` to report.
    fn tail_says_ours(path: &str) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut file) = File::open(path) else { return false };
        let Ok(length) = file.metadata().map(|m| m.len()) else { return false };
        let start = length.saturating_sub(Self::TAIL_PROBE);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return false;
        }
        let mut tail = Vec::with_capacity((length - start) as usize);
        if file.take(Self::TAIL_PROBE).read_to_end(&mut tail).is_err() {
            return false;
        }
        tail.windows(7).any(|w| w == b"/Pagify")
    }

    /// How much of a file's end is searched for the Secure Plus mark.
    const TAIL_PROBE: u64 = 1024 * 1024;

    /// Plaintext for a document sealed with Pagify's own handler, if it is one.
    ///
    /// `Ok(None)` means it is somebody else's document and should go to PDFium
    /// as it is. `Err` means it is ours and could not be opened — a missing or
    /// wrong password — which is reported in the same words PDFium uses for its
    /// own encryption, so a caller has one thing to handle rather than two.
    fn unseal_if_ours(bytes: &[u8], password: Option<&str>) -> Result<Option<Vec<u8>>> {
        // Cheap first: the handler's name is a fixed string, and parsing a
        // whole file to answer "is this ours?" for every open would be a cost
        // on every document that is not.
        if !bytes.windows(7).any(|w| w == b"/Pagify") {
            return Ok(None);
        }
        let Ok(file) = crate::pdf::File::parse(bytes) else { return Ok(None) };
        let Some(dict) = crate::pdf::secure_plus::dictionary_of(&file) else {
            return Ok(None);
        };

        let Some(password) = password.filter(|p| !p.is_empty()) else {
            return Err(PdfError::PasswordRequired);
        };
        let plus = crate::pdf::secure_plus::SecurePlus::open(password.as_bytes(), &dict)
            .map_err(|_| PdfError::IncorrectPassword)?;
        crate::pdf::secure_plus::unseal(&file, &plus).map(Some)
    }

    /// Adopt a descriptor detached from a `ParcelFileDescriptor` into an owning
    /// [`File`].
    ///
    /// Separated from [`PdfiumDocument::from_file`] so a caller can take ownership
    /// as its very first action. Everything that can fail afterwards then happens
    /// with the descriptor already owned by a value that closes it on drop, which
    /// leaves no window in which it could leak.
    ///
    /// # Safety
    /// `fd` must be an owned, readable, seekable descriptor that nothing else will
    /// close — i.e. it came from `ParcelFileDescriptor.detachFd()`, not `getFd()`.
    #[cfg(unix)]
    pub unsafe fn adopt_fd(fd: i32) -> Result<File> {
        use std::os::fd::FromRawFd;

        if fd < 0 {
            return Err(PdfError::InvalidArgument(format!(
                "file descriptor {fd} is not valid"
            )));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// Open from an already-owned file handle.
    ///
    /// This is the path that matters on Android: the document picker yields a
    /// `content://` URI, not a filesystem path, and streaming from its descriptor
    /// avoids copying a potentially very large file into the app's cache dir.
    ///
    /// Ownership of `file` is consumed unconditionally — on success the open
    /// document holds it (PDFium reads lazily throughout the document's life), and
    /// on failure it is dropped here, closing the descriptor. Callers must not
    /// close it themselves in either case.
    #[cfg(unix)]
    pub fn from_file(file: File, password: Option<&str>) -> Result<Self> {
        use std::os::fd::AsRawFd;

        let fd = file.as_raw_fd();
        Self::from_reader(file, password, DocumentSource::FileDescriptor(fd))
    }

    pub fn open_bytes(bytes: Vec<u8>, password: Option<&str>) -> Result<Self> {
        if let Some(unsealed) = Self::unseal_if_ours(&bytes, password)? {
            let mut doc = Self::open_bytes(unsealed, None)?;
            doc.already_secured = true;
            doc.secure_plus = true;
            doc.opened_with = password.map(|p| zeroize::Zeroizing::new(p.as_bytes().to_vec()));
            return Ok(doc);
        }
        // The same mending as `open_path`, for a document that arrives as bytes.
        let (bytes, repaired) = match locate_misplaced_xref_type(&bytes, 0) {
            Some((open, stray)) => {
                let mut bytes = bytes;
                move_xref_type(&mut bytes, open, stray);
                (bytes, true)
            }
            None => (bytes, false),
        };
        let byte_len = bytes.len();
        let mut doc = Self::from_reader(
            std::io::Cursor::new(bytes),
            password,
            DocumentSource::Memory { byte_len },
        )?;
        doc.repaired_on_open = repaired;
        Ok(doc)
    }

    fn from_reader<R: Read + Seek + 'static>(
        reader: R,
        password: Option<&str>,
        source: DocumentSource,
    ) -> Result<Self> {
        let document = pdfium()?
            .load_pdf_from_reader(reader, password)
            .map_err(|e| {
                let err = classify_pdfium_load_error(&e.to_string());
                // An empty password against an encrypted file is "password required"
                // (prompt the user) rather than "wrong password" (they mistyped).
                match (&err, password) {
                    (PdfError::IncorrectPassword, None) => PdfError::PasswordRequired,
                    _ => err,
                }
            })?;

        let page_count = document.pages().len() as usize;
        Ok(PdfiumDocument {
            document,
            source,
            page_count,
            dirty: false,
            vault: std::sync::Mutex::new(None),
            already_secured: password.is_some_and(|p| !p.is_empty()),
            remove_password: false,
            secure_plus: false,
            opened_with: password
                .filter(|p| !p.is_empty())
                .map(|p| zeroize::Zeroizing::new(p.as_bytes().to_vec())),
            security: None,
            redacted: false,
            written: None,
            exact_pending: false,
            exact_content: true,
            repaired_on_open: false,
            typing_fonts: Vec::new(),
            substituted: None,
            image_alpha: std::collections::HashMap::new(),
            next_alpha_id: 0,
            last_batch_timing: Vec::new(),
        })
    }

    pub fn source(&self) -> &DocumentSource {
        &self.source
    }

    /// Save through `FPDF_SaveWithVersion`, which is the only route to the
    /// incremental flag; the binding's own save hardcodes flags to zero.
    ///
    /// The `FPDF_FILEWRITE` callback is supplied here because the crate's
    /// equivalent is `pub(crate)`.
    /// The bytes PDFium would write with a given save flag.
    ///
    /// Public only so a probe can ask which `FPDF_REMOVE_SECURITY` value this
    /// build of PDFium uses — it is 3 in older releases and 4 in newer, and a
    /// wrong guess writes an encrypted file while believing it plain.
    pub fn save_flagged(&self, flags: u32) -> Result<Vec<u8>> {
        self.save_with_flags(flags)
    }

    fn save_with_flags(&self, flags: u32) -> Result<Vec<u8>> {
        #[repr(C)]
        struct Sink {
            base: FPDF_FILEWRITE,
            bytes: *mut Vec<u8>,
        }

        unsafe extern "C" fn write_block(
            this: *mut FPDF_FILEWRITE,
            data: *const c_void,
            size: c_ulong,
        ) -> c_int {
            let sink = this as *mut Sink;
            let out = &mut *(*sink).bytes;
            out.extend_from_slice(std::slice::from_raw_parts(data as *const u8, size as usize));
            1
        }

        let mut bytes: Vec<u8> = Vec::new();
        let mut sink = Sink {
            // PDFium hands the callback a pointer to the struct it was given, so
            // the interface header has to come first and the payload after it.
            base: FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            bytes: &mut bytes,
        };

        let ok = unsafe {
            pdfium()?.bindings().FPDF_SaveWithVersion(
                self.document.handle(),
                &mut sink as *mut Sink as *mut FPDF_FILEWRITE,
                // `FPDF_DWORD` is 32-bit on Windows and 64-bit on Android, so the
                // literal has to widen per target rather than be typed once.
                flags.into(),
                PDF_VERSION_1_7,
            )
        };
        if ok == 0 {
            return Err(PdfError::Pdfium("PDFium refused to save".into()));
        }
        Ok(bytes)
    }

    /// One page, as the bytes of a standalone single-page document.
    ///
    /// This is what makes a deletion undoable: PDFium destroys a page when it is
    /// deleted, so the content has to be taken out first, and a whole document is
    /// the only container the import side can read back.
    fn page_as_document_bytes(&self, page_index: i32) -> Result<Vec<u8>> {
        let mut scratch = pdfium()?
            .create_new_pdf()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        scratch
            .pages_mut()
            .copy_page_range_from_document(&self.document, page_index..=page_index, 0)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        scratch
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))
    }

    /// Render straight into an owned bitmap. Used by the prefetch path, which has
    /// no Android bitmap to draw into yet.
    pub fn render_page_to_bitmap(&self, index: usize, request: &RenderRequest) -> Result<Bitmap> {
        let page = self.page(index)?;
        let size = page.size();
        let (mut w, mut h) = size.pixel_size(request.scale);
        if request.rotation.swaps_axes() {
            std::mem::swap(&mut w, &mut h);
        }

        let mut bitmap = Bitmap::new(w, h, PixelOrder::Rgba)?;
        {
            let mut target = RenderTarget::from_bitmap(&mut bitmap)?;
            page.render_into(request, &mut target)?;
        }
        Ok(bitmap)
    }
}

impl Document for PdfiumDocument {
    fn backend_handle(&self) -> Option<usize> {
        Some(self.document.handle() as usize)
    }

    fn text_marks(&self, page_index: usize) -> Result<Vec<String>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let mut found = Vec::new();

        // Safety: as above.
        unsafe {
            for index in 0..bindings.FPDFPage_CountObjects(page.handle) {
                let object = bindings.FPDFPage_GetObject(page.handle, index);
                if object.is_null() || text_mark_id(bindings, object).is_none() {
                    continue;
                }
                // Only the first object of each caption carries the blob, so this
                // yields one entry per mark rather than one per letter.
                if let Some(blob) = text_mark_restore_of(bindings, object) {
                    found.push(blob);
                }
            }
        }

        Ok(found)
    }

    fn page_count(&self) -> usize {
        self.page_count
    }

    fn as_document_mut(&mut self) -> Option<&mut dyn DocumentMut> {
        Some(self)
    }

    fn metadata(&self) -> Result<DocumentMetadata> {
        use pdfium_render::prelude::PdfDocumentMetadataTagType as Tag;

        let mut meta = DocumentMetadata {
            page_count: self.page_count,
            ..Default::default()
        };

        let tags = self.document.metadata();
        for (tag, name) in [
            (Tag::Title, "Title"),
            (Tag::Author, "Author"),
            (Tag::Subject, "Subject"),
            (Tag::Keywords, "Keywords"),
            (Tag::Creator, "Creator"),
            (Tag::Producer, "Producer"),
            (Tag::CreationDate, "CreationDate"),
            (Tag::ModificationDate, "ModificationDate"),
        ] {
            if let Some(value) = tags.get(tag) {
                meta.set_tag(name, value.value());
            }
        }

        Ok(meta)
    }

    fn images_on(&self, page_index: usize) -> Result<Vec<crate::document::PageImage>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let space = raw.space()?;
        let bindings = pdfium()?.bindings();

        let mut found = Vec::new();
        for index in 0..unsafe { bindings.FPDFPage_CountObjects(raw.handle) } {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null()
                || unsafe { bindings.FPDFPageObj_GetType(handle) }
                    != pdfium_render::prelude::FPDF_PAGEOBJ_IMAGE as i32
            {
                continue;
            }

            let (mut left, mut bottom, mut right, mut top) = (0.0f32, 0.0, 0.0, 0.0);
            if unsafe {
                bindings.FPDFPageObj_GetBounds(handle, &mut left, &mut bottom, &mut right, &mut top)
            } == 0
            {
                continue;
            }

            let mut matrix = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
            unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut matrix) };

            // Metadata carries the pixel dimensions, which are not the size on
            // the page — a 2000px photo may be drawn two inches wide.
            let mut meta = pdfium_render::prelude::FPDF_IMAGEOBJ_METADATA {
                width: 0,
                height: 0,
                horizontal_dpi: 0.0,
                vertical_dpi: 0.0,
                bits_per_pixel: 0,
                colorspace: 0,
                marked_content_id: 0,
            };
            unsafe {
                bindings.FPDFImageObj_GetImageMetadata(handle, raw.handle, &mut meta);
            }

            let mut needed: c_ulong = 0;
            unsafe {
                needed = bindings.FPDFImageObj_GetImageDataRaw(
                    handle,
                    std::ptr::null_mut(),
                    0,
                ) as c_ulong;
            }

            let mut filters = Vec::new();
            for f in 0..unsafe { bindings.FPDFImageObj_GetImageFilterCount(handle) } {
                let length =
                    unsafe { bindings.FPDFImageObj_GetImageFilter(handle, f, std::ptr::null_mut(), 0) };
                if length <= 1 {
                    continue;
                }
                let mut buffer = vec![0u8; length as usize];
                unsafe {
                    bindings.FPDFImageObj_GetImageFilter(
                        handle,
                        f,
                        buffer.as_mut_ptr() as *mut c_void,
                        length,
                    )
                };
                // NUL-terminated, so the terminator is dropped rather than
                // becoming part of a name nothing will ever match.
                while buffer.last() == Some(&0) {
                    buffer.pop();
                }
                filters.push(String::from_utf8_lossy(&buffer).into_owned());
            }

            // Bounds arrive in PDF space, y up from the bottom-left; the rest of
            // this program works top-left down, so the same conversion every
            // other read here does.
            let top_left = space.to_top_left(left, top);
            let bottom_right = space.to_top_left(right, bottom);

            found.push(crate::document::PageImage {
                object: index as usize,
                rect: crate::document::Rect {
                    left: top_left.0,
                    top: top_left.1,
                    right: bottom_right.0,
                    bottom: bottom_right.1,
                },
                matrix: [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f],
                pixel_width: meta.width,
                pixel_height: meta.height,
                raw_bytes: needed as usize,
                filters,
            });
        }
        Ok(found)
    }

    fn run_font_data(&self, page_index: usize, object: usize) -> Result<Option<Vec<u8>>> {
        let (raw, handle) = match self.text_object_at(page_index, object)? {
            Some(found) => found,
            None => return Ok(None),
        };
        let bindings = pdfium()?.bindings();
        let font = unsafe { bindings.FPDFTextObj_GetFont(handle) };
        if font.is_null() {
            return Ok(None);
        }

        // Asked for the length first, as every sized PDFium read is. The
        // binding's `size_t` is `usize` and crate-private to pdfium-render, so
        // the alias is spelt out rather than imported.
        let mut needed: usize = 0;
        unsafe { bindings.FPDFFont_GetFontData(font, std::ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Ok(None);
        }
        let mut buffer = vec![0u8; needed];
        let read = unsafe {
            bindings.FPDFFont_GetFontData(font, buffer.as_mut_ptr(), needed, &mut needed)
        } != 0;
        drop(raw);
        if !read {
            return Ok(None);
        }
        buffer.truncate(needed);
        Ok(Some(buffer))
    }

    fn run_font_is_embedded(&self, page_index: usize, object: usize) -> Result<bool> {
        let Some((raw, handle)) = self.text_object_at(page_index, object)? else {
            return Ok(false);
        };
        let bindings = pdfium()?.bindings();
        let font = unsafe { bindings.FPDFTextObj_GetFont(handle) };
        let embedded = !font.is_null() && unsafe { bindings.FPDFFont_GetIsEmbedded(font) } == 1;
        drop(raw);
        Ok(embedded)
    }

    fn run_font_name(&self, page_index: usize, object: usize) -> Result<Option<String>> {
        let Some((raw, handle)) = self.text_object_at(page_index, object)? else {
            return Ok(None);
        };
        let bindings = pdfium()?.bindings();
        let font = unsafe { bindings.FPDFTextObj_GetFont(handle) };
        if font.is_null() {
            drop(raw);
            return Ok(None);
        }
        // Asked for the length first, as every sized PDFium read here is.
        let needed = unsafe { bindings.FPDFFont_GetBaseFontName(font, std::ptr::null_mut(), 0) };
        if needed == 0 {
            drop(raw);
            return Ok(None);
        }
        let mut buffer = vec![0u8; needed as usize];
        let written = unsafe {
            bindings.FPDFFont_GetBaseFontName(font, buffer.as_mut_ptr() as *mut c_char, needed)
        };
        drop(raw);
        if written == 0 {
            return Ok(None);
        }
        buffer.truncate(written as usize);
        // The count PDFium returns includes the trailing NUL.
        if buffer.last() == Some(&0) {
            buffer.pop();
        }
        let name = String::from_utf8_lossy(&buffer).into_owned();
        Ok((!name.is_empty()).then_some(name))
    }

    fn run_font_names(&self, page_index: usize) -> Result<std::collections::HashMap<usize, String>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };

        let mut names = std::collections::HashMap::new();
        for index in 0..count.max(0) {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null()
                || unsafe { bindings.FPDFPageObj_GetType(handle) }
                    != pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
            {
                continue;
            }
            let font = unsafe { bindings.FPDFTextObj_GetFont(handle) };
            if font.is_null() {
                continue;
            }
            let needed = unsafe { bindings.FPDFFont_GetBaseFontName(font, std::ptr::null_mut(), 0) };
            if needed == 0 {
                continue;
            }
            let mut buffer = vec![0u8; needed as usize];
            let written = unsafe {
                bindings.FPDFFont_GetBaseFontName(font, buffer.as_mut_ptr() as *mut c_char, needed)
            };
            if written == 0 {
                continue;
            }
            buffer.truncate(written as usize);
            if buffer.last() == Some(&0) {
                buffer.pop();
            }
            let name = String::from_utf8_lossy(&buffer).into_owned();
            if !name.is_empty() {
                names.insert(index as usize, name);
            }
        }
        Ok(names)
    }

    /// One page open and one walk over its objects, like `run_font_names`.
    ///
    /// **The font's identity is its PDFium handle**, which is one-to-one with
    /// the page's `/Font` resource — measured on the datasheet, where nine,
    /// eleven and nine handles came back for nine, eleven and nine resources,
    /// none merged and none split. The handle is an address, so it is only
    /// ever used as a key inside this call and handed out as a dense number.
    /// Its stem is probed once per handle however many objects share it.
    fn run_styles(
        &self,
        page_index: usize,
    ) -> Result<std::collections::HashMap<usize, crate::document::RunStyle>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };

        // font handle -> (the dense id it was given, its stem)
        let mut fonts: HashMap<usize, (u32, Option<u16>)> = HashMap::new();
        let mut styles = HashMap::new();
        for index in 0..count.max(0) {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null()
                || unsafe { bindings.FPDFPageObj_GetType(handle) }
                    != pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
            {
                continue;
            }
            let font = unsafe { bindings.FPDFTextObj_GetFont(handle) };
            if font.is_null() {
                continue;
            }
            let next = fonts.len() as u32;
            let (id, stem) = *fonts
                .entry(font as usize)
                .or_insert_with(|| (next, glyph_stem_milli_em(bindings, font)));

            // The text matrix, with the CTM already folded in by PDFium: its
            // x axis is the way the line runs. Normalised because a producer
            // that writes `1 Tf` carries the real size in it, and flipped in y
            // into the space `TextRun` uses (`0.0 - b`, not `-b`, so a level
            // line reads `0.0` and not `-0.0` in a log). The identity stands in
            // for a matrix that cannot be read or has collapsed to a point.
            let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
            let read = unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) } != 0;
            let length = m.a.hypot(m.b);
            let axis = if read && length.is_finite() && length > 1e-6 {
                (m.a / length, 0.0 - m.b / length)
            } else {
                (1.0, 0.0)
            };
            styles.insert(
                index as usize,
                crate::document::RunStyle { font: id, stem_milli_em: stem, axis },
            );
        }
        Ok(styles)
    }

    /// One page open and one question per object — its kind. Nothing is read:
    /// no word, no font, no box, no matrix. The page open is most of it (5 to 8
    /// ms on a datasheet page, 0.9 s on a villa drawing of 878,000 objects, which
    /// PDFium parses whole to open); the walk over the objects is a tenth of a
    /// microsecond apiece (a page of 40,000 words: 77 ms to open, 80 to count).
    fn page_scale(&self, page_index: usize) -> Result<crate::document::PageScale> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) }.max(0);
        let mut text_objects = 0usize;
        for index in 0..count {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if !handle.is_null()
                && unsafe { bindings.FPDFPageObj_GetType(handle) } == pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
            {
                text_objects += 1;
            }
        }
        Ok(crate::document::PageScale { text_objects, page_objects: count as usize })
    }

    /// Uses `FPDF_GetPageSizeByIndexF`, which reads the page tree without
    /// loading the page itself.
    fn text_runs(&self, page_index: usize) -> Result<Vec<crate::document::TextRun>> {
        Ok(self
            .text_runs_all(page_index)?
            .into_iter()
            .filter(|run| has_area(run.rect.left, run.rect.top, run.rect.right, run.rect.bottom))
            .collect())
    }

    /// `text_runs` without its area filter: see [`Document::text_runs_unfiltered`].
    fn text_runs_unfiltered(&self, page_index: usize) -> Result<Vec<crate::document::TextRun>> {
        self.text_runs_all(page_index)
    }

    /// Every text object's own bounding rect, without extracting its words —
    /// the hit-testing half of [`Self::text_runs`], for a caller (a click,
    /// most often) that only needs to know *where* things are, not what they
    /// say.
    ///
    /// **Why this exists.** `text_runs()` was almost entirely
    /// `PdfPageTextObject::text()` — a text layer loaded afresh for every run,
    /// seconds on a busy page; it now reads every run's words through the
    /// page's one layer (see `text_runs_all`) and takes tens of milliseconds —
    /// and a hit-test throws the words of every object except the one under
    /// the pointer away. On a page with a few hundred runs, clicking to select
    /// one was paying to extract the words of all the others too.
    fn text_run_rects(&self, page_index: usize) -> Result<Vec<(usize, Rect)>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let page = self
            .document
            .pages()
            .get(page_number)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let space = {
            let raw = RawPage::open(self.document.handle(), page_number)?;
            raw.space()?
        };

        let mut out = Vec::new();
        for (index, object) in page.objects().iter().enumerate() {
            if object.as_text_object().is_none() {
                continue;
            }
            // Same area filter as `text_runs()` — nothing to have clicked on
            // otherwise.
            let Ok(bounds) = object.bounds() else { continue };
            if !has_area(bounds.left().value, bounds.top().value, bounds.right().value, bounds.bottom().value) {
                continue;
            }
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);
            out.push((index, Rect { left, top, right, bottom }));
        }
        Ok(out)
    }

    /// One run, by its object number — the same fields [`Self::text_runs`]
    /// computes for every text object on the page, computed for just this
    /// one instead.
    ///
    /// **Why this exists at all.** `text_runs()` used to read the words of
    /// every text object on the page one `PdfPageTextObject::text()` call at a
    /// time — a text layer loaded afresh for each — and on a page of a few
    /// hundred runs that was most of a second, every single time it was asked
    /// for. It reads them through the page's one text layer now (tens of
    /// milliseconds), but a caller that already knows *which* object it wants
    /// (a click already resolved to one, a resize already has one selected)
    /// still has no use for the other few hundred answers, so it should not
    /// have to wait for them. **It is a page open and a text layer per call**
    /// — about 12 ms on the datasheet — so a caller after several runs wants
    /// [`Self::text_runs_some`]. `Option`, not an error, for the
    /// object existing but not being a run of real text — a picture, a
    /// shape, an index past the end — since none of those are a caller
    /// mistake worth a distinct message; `text_runs()` treats them the same
    /// way, by leaving them out of the list.
    fn text_run_at(&self, page_index: usize, object: usize) -> Result<Option<crate::document::TextRun>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = self
            .document
            .pages()
            .get(page_number)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        // Same "read the matrix, then drop the raw handle" shape as
        // `text_runs()`, for the same reason — see its own comment.
        let (space, origin) = {
            let raw = RawPage::open(self.document.handle(), page_number)?;
            let space = raw.space()?;
            let bindings = pdfium()?.bindings();
            let index = i32::try_from(object).unwrap_or(-1);
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null() {
                return Ok(None);
            }
            let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
            let read = unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) } != 0;
            (space, read.then(|| space.to_top_left(m.e, m.f)))
        };

        let Ok(object_ref) = page.objects().get(object) else { return Ok(None) };
        let Some(text_object) = object_ref.as_text_object() else { return Ok(None) };
        // The same machinery `text_runs()` reads a run's words with — see
        // its own comment on why this is kept even when it reports none.
        let text_page = page.text().map_err(|e| PdfError::Pdfium(e.to_string()))?;
        // Through the text layer just loaded, not `text_object.text()`, which
        // loads one of its own on every call — see `text_runs_all`.
        let words = text_page.for_object(text_object);
        drop(text_page);

        let Ok(bounds) = object_ref.bounds() else { return Ok(None) };
        if !has_area(bounds.left().value, bounds.top().value, bounds.right().value, bounds.bottom().value) {
            return Ok(None);
        }
        let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
        let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);

        let colour = object_ref
            .fill_color()
            .map(|c| Color { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() })
            .unwrap_or(Color { r: 0, g: 0, b: 0, a: 255 });
        let origin = match origin {
            Some((x, y)) => Point { x, y },
            None => Point { x: left, y: bottom },
        };

        Ok(Some(crate::document::TextRun {
            object,
            text: words,
            rect: Rect { left, top, right, bottom },
            origin,
            // The *effective* size, as `text_runs_all` and `text_runs_some` give
            // it — not the bare `Tf` size. Every caller that divides a requested
            // size by `run.size / found.size` (see `set_run_in_stream`'s
            // `vertical_scale`) needs the effective one in `run.size`: for a
            // producer that writes `1 Tf` and puts the size in the matrix, the
            // bare one made that ratio 1 and a requested 9 pt a `9 Tf` under a
            // 9x matrix.
            size: text_object.scaled_font_size().value,
            color: colour,
        }))
    }

    /// Same walk as [`Self::text_run_at`] repeated per object costs a page
    /// open and a text layer on every call (about 12 ms each on the
    /// datasheet), more than reading the whole page once does. This instead
    /// opens the page once, the same as `text_runs_all`, and reads the words
    /// — through the page's one text layer — of only the objects actually in
    /// `wanted`.
    fn text_runs_some(
        &self,
        page_index: usize,
        wanted: &std::collections::HashSet<usize>,
    ) -> Result<Vec<crate::document::TextRun>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let page = self
            .document
            .pages()
            .get(page_number)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let (space, origins) = {
            let raw = RawPage::open(self.document.handle(), page_number)?;
            let space = raw.space()?;
            let bindings = pdfium()?.bindings();
            let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
            let mut origins = Vec::with_capacity(count.max(0) as usize);
            for index in 0..count.max(0) {
                let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
                let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
                let read = !handle.is_null()
                    && unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) } != 0;
                origins.push(read.then(|| space.to_top_left(m.e, m.f)));
            }
            (space, origins)
        };
        let text_page = page.text().map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let mut runs = Vec::new();
        for (index, object) in page.objects().iter().enumerate() {
            if !wanted.contains(&index) {
                continue;
            }
            let Some(text_object) = object.as_text_object() else { continue };
            let words = text_page.for_object(text_object);
            let Ok(bounds) = object.bounds() else { continue };
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);
            let colour = object
                .fill_color()
                .map(|c| Color { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() })
                .unwrap_or(Color { r: 0, g: 0, b: 0, a: 255 });
            let origin = match origins.get(index).copied().flatten() {
                Some((x, y)) => Point { x, y },
                None => Point { x: left, y: bottom },
            };
            runs.push(crate::document::TextRun {
                object: index,
                text: words,
                rect: Rect { left, top, right, bottom },
                origin,
                size: text_object.scaled_font_size().value,
                color: colour,
            });
        }
        drop(text_page);
        Ok(runs)
    }

    fn take_last_batch_timing(&mut self) -> Vec<(&'static str, std::time::Duration)> {
        std::mem::take(&mut self.last_batch_timing)
    }

    fn annotations(&self, page_index: usize) -> Result<Vec<IndexedAnnotation>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_GetAnnotCount(page.handle) };

        let mut marks = Vec::new();
        for i in 0..count.max(0) {
            let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, i) };
            if annot.is_null() {
                continue;
            }
            let read = self.read_annotation(annot, &space);
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };

            // The index carried is PDFium's, not this list's. A page holding one
            // form widget followed by one highlight yields a single entry whose
            // index is 1 — addressing it as 0 would delete the widget.
            if let Some(annotation) = read? {
                marks.push(IndexedAnnotation {
                    index: i as usize,
                    annotation,
                });
            }
        }
        Ok(marks)
    }

    fn internal_links(&self, page_index: usize) -> Result<Vec<InternalLink>> {
        self.validate_page_index(page_index)?;
        let number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let space = RawPage::open(self.document.handle(), number)?.space()?;
        let page = self
            .document
            .pages()
            .get(number)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let mut out = Vec::new();
        for link in page.links().iter() {
            // A link names its place either directly (`/Dest`) or through a
            // `/GoTo` action; a `/URI`, a launch or a link to another file has
            // neither and is not this method's to report.
            let target = link.destination().and_then(|d| d.page_index().ok()).or_else(|| {
                let action = link.action()?;
                let local = action.as_local_destination_action()?;
                local.destination().ok()?.page_index().ok()
            });
            let Some(target) = target else { continue };
            let Ok(bounds) = link.rect() else { continue };
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);
            out.push(InternalLink {
                rect: Rect {
                    left: left.min(right),
                    top: top.min(bottom),
                    right: left.max(right),
                    bottom: top.max(bottom),
                },
                page: target as usize,
            });
        }
        Ok(out)
    }

    fn bookmarks(&self) -> Result<Vec<(String, usize)>> {
        let mut out = Vec::new();
        // `root()` is not a synthetic container — PDFium's own
        // `FPDFBookmark_GetFirstChild(doc, NULL)` is what a NULL parent
        // means, so this already *is* the first top-level bookmark, and its
        // siblings are the rest. `iter()`/`iter_all_descendants()` would
        // also walk into children, which `add_bookmark` never creates but a
        // document imported from elsewhere might have — this method's own
        // doc promises top level only.
        let Some(mut current) = self.document.bookmarks().root() else {
            return Ok(out);
        };
        loop {
            if let (Some(title), Some(destination)) = (current.title(), current.destination()) {
                if let Ok(page_index) = destination.page_index() {
                    out.push((title, page_index as usize));
                }
            }
            match current.next_sibling() {
                Some(next) => current = next,
                None => break,
            }
        }
        Ok(out)
    }

    fn permissions(&self) -> Option<crate::pdf::encrypt::Permissions> {
        // A password chosen and not yet written is the one that will be in
        // force; reporting what the file says instead would describe the
        // document somebody had rather than the one they have.
        if let Some(wanted) = &self.security {
            return Some(wanted.permissions);
        }
        if !self.already_secured {
            return None;
        }
        let bits = unsafe { pdfium().ok()?.bindings().FPDF_GetDocPermissions(self.document.handle()) };
        Some(crate::pdf::encrypt::Permissions(bits as u32))
    }

    /// **Worth saying out loud.** Words written in a face that is not their
    /// neighbours' look like what they are, and somebody who is not told will
    /// find out from a printed page.
    fn substituted_face(&self) -> Option<String> {
        self.substituted.clone()
    }

    fn repaired_on_open(&self) -> bool {
        self.repaired_on_open
    }


    fn move_object(&mut self, page_index: usize, object: usize, by: Point) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let index = i32::try_from(object).map_err(|_| {
            PdfError::InvalidArgument(format!("object {object} is out of range"))
        })?;

        // **A picture moves without PDFium's help.** Its drawing is one
        // operator, and wrapping that operator cannot reach anything else — so
        // there is no re-emission to guard against. See
        // `move_picture_in_stream`; anything it cannot follow falls through to
        // the guarded path below.
        if self.images_on(page_index)?.iter().any(|i| i.object == object) {
            match self.move_picture_in_stream(page_index, object, by) {
                Ok(()) => return Ok(()),
                Err(PdfError::Unsupported(_)) => {}
                Err(other) => return Err(other),
            }
        }

        // **And so does a run of words.** Shifting the text matrix its
        // operators draw under moves them and reaches nothing else, so this
        // needs no re-emission either — see `move_run_in_stream` for the two
        // operators it inserts and the cases it declines. What it declines
        // falls through to the guarded path below, which still works and still
        // refuses on a page it would rewrite.
        //
        // **`text_run_at`, not `text_runs().iter().any(...)`.** The full scan
        // was exactly the cost `text_run_at` exists to avoid — asking "is
        // this object a run of text" the expensive way, then answering the
        // real question the cheap way right after, on a page with a few
        // hundred runs paid for the whole list twice for no reason. Measured
        // on a real page: this gate alone was ~800ms of what looked like a
        // "move" taking a few seconds.
        if self.text_run_at(page_index, object)?.is_some() {
            match self.move_run_in_stream(page_index, object, by) {
                Ok(()) => return Ok(()),
                Err(PdfError::Unsupported(_)) => {}
                Err(other) => return Err(other),
            }
        }

        // **And so does a drawn shape.** Its operators are pure graphics, so
        // `q`/`Q` around them is legal and reaches nothing else — see
        // `move_path_in_stream`.
        if self.path_ordinal(page_index, object)?.is_some() {
            match self.move_path_in_stream(page_index, object, by) {
                Ok(()) => return Ok(()),
                Err(PdfError::Unsupported(_)) => {}
                Err(other) => return Err(other),
            }
        }

        // **What the page says now, and a way back to it.**
        //
        // Committing a move needs `FPDFPage_GenerateContent`, which re-emits
        // the whole content stream — and on a real catalogue that came back
        // scrambled: `HSI Lighting I HUE . SATURATION` as
        // `ABOThe Boer T UOur e xpiunpics`, with the page's vertically-set
        // heading broken into pieces. Measured after it had already been
        // offered, which is the wrong order and the reason this guard exists.
        //
        // The words are read before and after, and a move that changes any of
        // them is undone. A refusal costs a save-and-reopen; the alternative
        // costs somebody their document.
        let before: Vec<String> = self
            .text_runs(page_index)?
            .iter()
            .map(|r| r.text.trim().to_string())
            .collect();
        // **And what is on the page besides its words.** A page with no text
        // compares nothing with nothing, so the words alone let a re-emission
        // through on exactly the pages that have no other protection — see
        // `object_census`.
        let census_before = self.object_census(page_index)?;
        let snapshot = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        // Page space counts downwards and a content stream upwards, so a move
        // *down* the screen is a move down in y here and up in the file.
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
        if handle.is_null() {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has no object {object}",
                page_index + 1
            )));
        }
        unsafe { bindings.FPDFPageObj_Transform(handle, 1.0, 0.0, 0.0, 1.0, by.x as f64, -by.y as f64) };
        let generated = unsafe { bindings.FPDFPage_GenerateContent(raw.handle) } != 0;
        drop(raw);
        if !generated {
            return Err(PdfError::Pdfium("the page could not be rewritten".into()));
        }

        let after: Vec<String> = self
            .text_runs(page_index)?
            .iter()
            .map(|r| r.text.trim().to_string())
            .collect();
        let (mut was, mut now) = (before.clone(), after.clone());
        was.sort();
        now.sort();

        // Every object still there, still its own kind, and still where it was
        // — bar the one that was asked to move, which has to have moved by
        // exactly what was asked.
        let census_after = self.object_census(page_index)?;
        let lost = census_after.len() != census_before.len()
            || census_before.iter().zip(&census_after).enumerate().any(|(at, (a, b))| {
                // Kind and colour must match exactly; position must match bar
                // the move that was asked for. Order is checked by comparing
                // index against index: a page whose objects came back
                // re-stacked has a different kind or a different rectangle at
                // some index, which is what a picture sinking behind its
                // neighbours looks like from here.
                if a.kind != b.kind || a.fill != b.fill || a.stroke != b.stroke {
                    return true;
                }
                let (dx, dy) = if at == object { (by.x, by.y) } else { (0.0, 0.0) };
                (b.rect.left - a.rect.left - dx).abs() > 0.5
                    || (b.rect.top - a.rect.top - dy).abs() > 0.5
            });

        if was != now || lost {
            // Put the page back exactly as it was and say why.
            let restored = Self::open_bytes(snapshot, None)?;
            self.document = restored.document;
            self.page_count = restored.page_count;
            if let Ok(mut cached) = self.vault.lock() {
                *cached = None;
            }
            return Err(PdfError::Unsupported(if was == now {
                "moving anything on this page rewrites what is drawn on it, so nothing was moved"
            } else {
                "moving anything on this page rewrites its text, so nothing was moved"
            }));
        }

        self.touch();
        Ok(())
    }

    /// Take one picture, shape or run of words off the page — its own
    /// drawing operators spliced out, not covered by anything. Reuses
    /// exactly the object-locating logic [`Self::object_wrap_site`] does for
    /// move/resize (same picture-frame/placeholder handling, same path and
    /// run lookups), but never needs a *wrapping* scope the way a geometric
    /// transform does, so it is not refused for a run sharing a text box —
    /// deleting the run's own operators does not touch its neighbours either
    /// way.
    fn remove_object(&mut self, page_index: usize, object: usize) -> Result<()> {
        use crate::pdf::content;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        let span: std::ops::Range<usize> = 'span: {
            let pictures = self.images_on(page_index)?;
            if let Some(which) = pictures.iter().position(|i| i.object == object) {
                let names = self.image_names(&file, &page)?;
                let drawn = image_operators(&operations, &names);
                if drawn.len() != pictures.len() {
                    return Err(PdfError::Unsupported(
                        "this page draws its pictures in a way this cannot follow",
                    ));
                }
                let at = drawn[which];
                break 'span match frame_scope(&operations, at..at + 1) {
                    Some((open, close)) => match placeholder_before(&operations, open) {
                        Some(first) => first..close + 1,
                        None => open..close + 1,
                    },
                    None => at..at + 1,
                };
            }

            if let Some((which, paths)) = self.path_ordinal(page_index, object)? {
                let painted = path_operators(&operations);
                if painted.len() != paths {
                    return Err(PdfError::Unsupported(
                        "this page paints its shapes in a way this cannot follow",
                    ));
                }
                let (pspan, clips) = painted
                    .get(which)
                    .cloned()
                    .ok_or(PdfError::Unsupported("that shape is not painted on this page"))?;
                if clips {
                    return Err(PdfError::Unsupported(
                        "this shape also sets a clipping path, which cannot be removed alone",
                    ));
                }
                break 'span match frame_scope(&operations, pspan.clone()) {
                    Some((open, close)) => open..close + 1,
                    None => pspan,
                };
            }

            if let Some(run) = self.text_run_at(page_index, object)? {
                let height = self.page_size(page_index)?.height_pt;
                let fonts = self.page_fonts(&file, &page);
                let codes_in = |p: &content::Placed| -> usize {
                    let width = p
                        .font
                        .as_ref()
                        .zip(fonts.as_ref())
                        .and_then(|(name, dict)| code_width(&file, dict, name))
                        .unwrap_or(1)
                        .max(1);
                    content::pieces(&operations[p.origin.operation])
                        .iter()
                        .map(|piece| match piece {
                            content::Piece::Codes(bytes) => bytes.len() / width,
                            content::Piece::Kern(_) => 0,
                        })
                        .sum()
                };
                let order = self.text_order(page_index, object);
                let (first, last, _) =
                    run_operators(&run, height, &placed, &operations, &codes_in, order)?;
                break 'span first..last + 1;
            }

            return Err(PdfError::Unsupported("that is not something this can remove"));
        };

        let from = operations[span.start].span.start;
        let to = operations[span.end - 1].span.end;
        let edited = content::splice(&stream, &[(from..to, Vec::new())]);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// See [`crate::document::Document::remove_objects`].
    ///
    /// [`Self::remove_object`] run once per object costs a full
    /// `save_to_bytes` + content-stream reparse *per object* — the thing
    /// this exists to avoid (see that method's doc comment, and
    /// [`crate::command::Command::RemoveObjects`]). Everything expensive —
    /// `edit_base`, `File::parse`, the content stream, its parsed operations
    /// and the page's picture/shape operator tables — is done exactly once
    /// here, up front; only the per-object span lookup (which of those
    /// already-parsed operators belongs to *this* object) repeats per
    /// object. One `content::splice` call removes every span at once, and
    /// one write-back commits the result.
    fn remove_objects(&mut self, page_index: usize, objects: &[usize]) -> Result<()> {
        use crate::pdf::content;

        if objects.is_empty() {
            return Ok(());
        }

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        let pictures = self.images_on(page_index)?;
        let names = self.image_names(&file, &page)?;
        let drawn = image_operators(&operations, &names);

        let painted = path_operators(&operations);

        let height = self.page_size(page_index)?.height_pt;
        let fonts = self.page_fonts(&file, &page);
        let codes_in = |p: &content::Placed| -> usize {
            let width = p
                .font
                .as_ref()
                .zip(fonts.as_ref())
                .and_then(|(name, dict)| code_width(&file, dict, name))
                .unwrap_or(1)
                .max(1);
            content::pieces(&operations[p.origin.operation])
                .iter()
                .map(|piece| match piece {
                    content::Piece::Codes(bytes) => bytes.len() / width,
                    content::Piece::Kern(_) => 0,
                })
                .sum()
        };

        let mut byte_ranges: Vec<std::ops::Range<usize>> = Vec::with_capacity(objects.len());

        for &object in objects {
            let span: std::ops::Range<usize> = 'span: {
                if let Some(which) = pictures.iter().position(|i| i.object == object) {
                    if drawn.len() != pictures.len() {
                        return Err(PdfError::Unsupported(
                            "this page draws its pictures in a way this cannot follow",
                        ));
                    }
                    let at = drawn[which];
                    break 'span match frame_scope(&operations, at..at + 1) {
                        Some((open, close)) => match placeholder_before(&operations, open) {
                            Some(first) => first..close + 1,
                            None => open..close + 1,
                        },
                        None => at..at + 1,
                    };
                }

                if let Some((which, paths)) = self.path_ordinal(page_index, object)? {
                    if painted.len() != paths {
                        return Err(PdfError::Unsupported(
                            "this page paints its shapes in a way this cannot follow",
                        ));
                    }
                    let (pspan, clips) = painted
                        .get(which)
                        .cloned()
                        .ok_or(PdfError::Unsupported("that shape is not painted on this page"))?;
                    if clips {
                        return Err(PdfError::Unsupported(
                            "this shape also sets a clipping path, which cannot be removed alone",
                        ));
                    }
                    break 'span match frame_scope(&operations, pspan.clone()) {
                        Some((open, close)) => open..close + 1,
                        None => pspan,
                    };
                }

                if let Some(run) = self.text_run_at(page_index, object)? {
                    let order = self.text_order(page_index, object);
                    let (first, last, _) =
                        run_operators(&run, height, &placed, &operations, &codes_in, order)?;
                    break 'span first..last + 1;
                }

                return Err(PdfError::Unsupported("that is not something this can remove"));
            };

            let from = operations[span.start].span.start;
            let to = operations[span.end - 1].span.end;
            byte_ranges.push(from..to);
        }

        let edits: Vec<(std::ops::Range<usize>, Vec<u8>)> =
            byte_ranges.into_iter().map(|r| (r, Vec::new())).collect();
        let edited = content::splice(&stream, &edits);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// See [`crate::document::Document::split_run_into_characters`].
    ///
    /// Each character keeps the run's own font, size and rotation — only
    /// its own `Tm` translation changes, taken from PDFium's own
    /// `FPDFText_GetCharOrigin` for that exact character rather than
    /// anything computed here, so no font's metrics need reimplementing to
    /// get this right. A `Tm` restoring the line matrix afterward — the same
    /// reason [`Self::move_run_in_stream`]'s non-continuing branch writes
    /// one — is what keeps whatever repositions itself after this run (a
    /// `Td` starting the next line) computing from where that line always
    /// started, not from wherever the last character landed.
    fn split_run_into_characters(&mut self, page_index: usize, object: usize) -> Result<()> {
        use crate::pdf::content;

        let run = self
            .text_run_at(page_index, object)?
            .ok_or_else(|| PdfError::InvalidArgument("that is not a run of text".into()))?;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);
        let states = content::states(&operations);
        let height = self.page_size(page_index)?.height_pt;

        let fonts = self.page_fonts(&file, &page);
        let codes_in = |p: &content::Placed| -> usize {
            let width = p
                .font
                .as_ref()
                .zip(fonts.as_ref())
                .and_then(|(name, dict)| code_width(&file, dict, name))
                .unwrap_or(1)
                .max(1);
            content::pieces(&operations[p.origin.operation])
                .iter()
                .map(|piece| match piece {
                    content::Piece::Codes(bytes) => bytes.len() / width,
                    content::Piece::Kern(_) => 0,
                })
                .sum()
        };
        let order = self.text_order(page_index, object);
        let (first, last, continues) =
            run_operators(&run, height, &placed, &operations, &codes_in, order)?;
        if continues {
            return Err(PdfError::Unsupported(
                "more text is drawn right after these words on the same line with nothing \
                 repositioning in between, so splitting these into characters",
            ));
        }

        let governing = placed
            .iter()
            .find(|p| p.origin.operation == first)
            .ok_or(PdfError::Unsupported("that text selects no font"))?;
        let font_name =
            governing.font.clone().ok_or(PdfError::Unsupported("that text selects no font"))?;
        let width = fonts
            .as_ref()
            .and_then(|dict| code_width(&file, dict, &font_name))
            .unwrap_or(1)
            .max(1);

        let mut code_bytes = Vec::new();
        for index in first..=last {
            for piece in content::pieces(&operations[index]) {
                if let content::Piece::Codes(b) = piece {
                    code_bytes.extend_from_slice(&b);
                }
            }
        }
        let n = code_bytes.len() / width;
        if n <= 1 {
            // Already one character, or nothing drawn — nothing to split.
            return Ok(());
        }

        let ctm = states[first].ctm;
        let Some(inv) = inverse(ctm) else {
            return Err(PdfError::Unsupported(
                "these words are placed by a transform this cannot invert",
            ));
        };
        let rotation = states[first].text;

        let page_number = i32::try_from(page_index)
            .map_err(|_| PdfError::PageOutOfRange { index: page_index, count: self.page_count })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let text_page = unsafe { bindings.FPDFText_LoadPage(raw.handle) };
        if text_page.is_null() {
            return Err(PdfError::Unsupported("this page's text cannot be read to split it"));
        }
        let total_chars = unsafe { bindings.FPDFText_CountChars(text_page) };

        // Which of PDFium's own characters are this run's: by who drew
        // them, not by counting up to them. Its own text extraction quietly
        // inserts a synthetic space between many separate runs
        // (`FPDFText_IsGenerated`) that is not a byte anywhere in the
        // content stream, so counting content-stream codes up to this run's
        // first operator drifts further out of step with PDFium's own
        // numbering the more such runs sit earlier on the page — on a busy
        // page, by enough to land on someone else's characters entirely.
        // Asking PDFium which page object drew each one instead cannot
        // drift, because it is the same answer PDFium already gave `object`
        // itself.
        //
        // **Only the show-text operators in `first..=last` get their own
        // page object** — a `Tf` or a colour change in between does not —
        // so counting the whole span, not just those, once named the wrong
        // objects entirely on a run whose own operators were not the only
        // ones in it.
        let operator_count = operations[first..=last].iter().filter(|op| op.shows_text()).count();
        let mut owners = Vec::with_capacity(operator_count);
        for k in 0..operator_count {
            let index = i32::try_from(object + k)
                .map_err(|_| PdfError::InvalidArgument(format!("object {object} is out of range")))?;
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null() {
                unsafe { bindings.FPDFText_ClosePage(text_page) };
                return Err(PdfError::Unsupported(
                    "this run's own objects could not all be found to split",
                ));
            }
            owners.push(handle);
        }
        let mut char_indices = Vec::with_capacity(n);
        for index in 0..total_chars {
            if unsafe { bindings.FPDFText_IsGenerated(text_page, index) } == 1 {
                continue;
            }
            let owner = unsafe { bindings.FPDFText_GetTextObject(text_page, index) };
            if owners.contains(&owner) {
                char_indices.push(index);
            }
        }
        if char_indices.len() != n {
            unsafe { bindings.FPDFText_ClosePage(text_page) };
            return Err(PdfError::Unsupported(
                "this run's characters could not all be matched to split",
            ));
        }

        let mut written = Vec::new();
        for (i, &index) in char_indices.iter().enumerate() {
            let (mut x, mut y) = (0.0f64, 0.0f64);
            let ok = unsafe { bindings.FPDFText_GetCharOrigin(text_page, index, &mut x, &mut y) } != 0;
            if !ok {
                unsafe { bindings.FPDFText_ClosePage(text_page) };
                return Err(PdfError::Unsupported(
                    "one of these characters could not be found to split",
                ));
            }
            // The local translation that puts this character's own `Tm` at
            // exactly the absolute page position PDFium reports for it — the
            // same inverse-CTM step `move_run_in_stream` takes for a move,
            // just landing on this character's own spot instead of a
            // shifted one.
            let (ax, ay) = (x as f32, y as f32);
            let local = (inv[0] * ax + inv[2] * ay + inv[4], inv[1] * ax + inv[3] * ay + inv[5]);
            let mut tm = rotation;
            tm[4] = local.0;
            tm[5] = local.1;

            let code = &code_bytes[i * width..(i + 1) * width];
            written.extend_from_slice(&text_matrix(&tm)?);
            written.push(b'\n');
            written.extend_from_slice(&hex_string_operand(code));
            written.extend_from_slice(b" Tj\n");
        }
        unsafe { bindings.FPDFText_ClosePage(text_page) };

        // Whatever comes after this run finds the line exactly where it
        // would have without the split.
        written.extend_from_slice(&text_matrix(&states[last].line)?);

        let from = operations[first].span.start;
        let to = operations[last].span.end;
        let edited = content::splice(&stream, &[(from..to, written)]);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    fn drawn_objects(&self, page_index: usize) -> Result<Vec<crate::document::DrawnObject>> {
        self.drawn_walk(page_index, false)
    }

    /// See [`Document::drawn_shapes`]: the same walk with nothing read that a
    /// shape's entry does not hold.
    fn drawn_shapes(&self, page_index: usize) -> Result<Vec<crate::document::DrawnObject>> {
        self.drawn_walk(page_index, true)
    }

    /// Every drawn object of a page — or, with `shapes_only`, just its paths.
    ///
    /// **`shapes_only` returns the very entries the full list holds for its
    /// paths.** What it skips is what no path's entry contains: the text of every
    /// text object (read through the page's text layer, which is a pass over the
    /// whole page's characters *per object* — the cost of the full list grows
    /// with the square of the page's text, until a page has more than
    /// [`LINEAR_TEXT_READ_FROM`] text objects and the labels are read in one pass
    /// as `text_runs_unfiltered`'s words are), the pictures' pixel sizes and
    /// their opacity, and a text object's or a group's own box and opacity. Everything
    /// that decides a path's entry is kept: the walk order, the forms stepped
    /// into, each path's box and opacity, and the placeholder rule that moves a
    /// grey box under the picture it backs — which looks at the kinds and boxes
    /// of neighbours, so a picture's box is read and the rest of the list's
    /// entries are still made. `tests/page_weight.rs` holds the two lists to each
    /// other on a page with all of it.
    fn drawn_walk(&self, page_index: usize, shapes_only: bool) -> Result<Vec<crate::document::DrawnObject>> {
        use crate::document::{DrawnKind, DrawnObject};
        use pdfium_render::prelude::{
            FPDF_PAGEOBJ_FORM, FPDF_PAGEOBJ_IMAGE, FPDF_PAGEOBJ_PATH, FPDF_PAGEOBJ_SHADING,
            FPDF_PAGEOBJ_TEXT,
        };

        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let space = raw.space()?;
        let bindings = pdfium()?.bindings();
        // Open once for the whole walk: `FPDFTextObj_GetText` needs one, and
        // opening a text page per object on a busy page is the difference
        // between a list and a wait. Not at all for the shapes: no path's entry
        // has words in it.
        let text_page = if shapes_only { std::ptr::null_mut() } else { unsafe { bindings.FPDFText_LoadPage(raw.handle) } };

        // The pictures, for their pixel size — which is the useful thing to say
        // about one in a list, and is not on the object.
        let pictures = if shapes_only { Vec::new() } else { self.images_on(page_index).unwrap_or_default() };

        /// How far in this will go. A form inside a form inside a form is real;
        /// an unbounded walk of a malformed one is a hang.
        const DEEPEST: usize = 4;

        let mut out: Vec<DrawnObject> = Vec::new();
        // **An object inside a form is measured in the form's own space.**
        // PDFium reports its bounds there, not on the page — a picture
        // drawn at the top of a form placed low on the page read as sitting
        // at the top of the page. Each entry carries the matrix that takes
        // its space to the page's: the form objects' matrices, innermost
        // first, which is what the page's own objects have as identity.
        let mut stack: Vec<(FPDF_PAGEOBJECT, usize, usize, [f32; 6])> = Vec::new();
        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
        // The page's own text objects, for the labels of a page large enough that
        // reading each one's words by itself is the slow part (see
        // [`LINEAR_TEXT_READ_FROM`]).
        let mut own_text: Vec<(usize, FPDF_PAGEOBJECT)> = Vec::new();
        // Pushed backwards so the first object is taken first.
        for index in (0..count).rev() {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if !handle.is_null() {
                stack.push((handle, 0, index as usize, IDENTITY_MATRIX));
                if !shapes_only && unsafe { bindings.FPDFPageObj_GetType(handle) } == FPDF_PAGEOBJ_TEXT as i32 {
                    own_text.push((index as usize, handle));
                }
            }
        }
        // `None` for a small page, which asks PDFium object by object; and for any
        // object inside a form, which the page's own characters do not name.
        let read_in_one_pass = (own_text.len() > LINEAR_TEXT_READ_FROM)
            .then(|| words_by_characters(bindings, raw.handle, &own_text))
            .flatten();

        while let Some((handle, depth, top, to_page)) = stack.pop() {
            let kind_code = unsafe { bindings.FPDFPageObj_GetType(handle) };
            let kind = match kind_code as u32 {
                FPDF_PAGEOBJ_TEXT => DrawnKind::Words,
                FPDF_PAGEOBJ_IMAGE => DrawnKind::Picture,
                FPDF_PAGEOBJ_PATH => DrawnKind::Shape,
                FPDF_PAGEOBJ_FORM | FPDF_PAGEOBJ_SHADING => DrawnKind::Group,
                _ => DrawnKind::Group,
            };

            let (mut l, mut b, mut r, mut t) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            // Only a path's entry holds a box that is read back, and the
            // placeholder rule below compares one with a picture's; a text
            // object's and a group's are not asked for when only the paths are
            // wanted.
            let wants_box = !shapes_only || matches!(kind, DrawnKind::Shape | DrawnKind::Picture);
            let rect = if !wants_box
                || unsafe { bindings.FPDFPageObj_GetBounds(handle, &mut l, &mut b, &mut r, &mut t) } == 0
            {
                Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
            } else {
                let (l, b, r, t) = bounds_through(l, b, r, t, to_page);
                let (left, top_pt) = space.to_top_left(l, t);
                let (right, bottom) = space.to_top_left(r, b);
                Rect { left, top: top_pt, right, bottom }
            };

            let label = match kind {
                DrawnKind::Words | DrawnKind::Picture if shapes_only => String::new(),
                DrawnKind::Words => {
                    let words = match &read_in_one_pass {
                        Some(read) if depth == 0 => read.get(&top).cloned().unwrap_or_default(),
                        _ => object_text(bindings, text_page, handle),
                    };
                    let words = words.trim();
                    let short: String = words.chars().take(40).collect();
                    if words.chars().count() > 40 {
                        format!("{short}…")
                    } else if short.is_empty() {
                        "(blank)".to_string()
                    } else {
                        short
                    }
                }
                DrawnKind::Picture => pictures
                    .iter()
                    .find(|i| i.object == top && depth == 0)
                    .map(|i| {
                        let filter = i.filters.first().map(String::as_str).unwrap_or("raw");
                        format!("{}×{} {}", i.pixel_width, i.pixel_height, filter)
                    })
                    .unwrap_or_else(|| {
                        format!("{:.0} × {:.0} pt", rect.right - rect.left, rect.bottom - rect.top)
                    }),
                DrawnKind::Shape | DrawnKind::Group => {
                    format!("{:.0} × {:.0} pt", rect.right - rect.left, rect.bottom - rect.top)
                }
            };

            // The fill alpha is the object's own opacity as PDFium resolved it
            // — the `ca` in force when it was drawn. Only a path's is wanted
            // when only paths are.
            let opacity = if shapes_only && kind != DrawnKind::Shape {
                1.0
            } else {
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 255u32);
                if unsafe { bindings.FPDFPageObj_GetFillColor(handle, &mut r, &mut g, &mut b, &mut a) }
                    == 0
                {
                    a = 255;
                }
                a as f32 / 255.0
            };

            out.push(DrawnObject {
                object: top,
                kind,
                rect,
                label,
                opacity,
                depth,
                // Only what the page itself draws can be re-ordered — see
                // `DrawnObject::movable`.
                movable: depth == 0,
            });

            // What a group draws goes in straight after it, so the list reads
            // as the page draws: the group, then its contents on top of it.
            if kind_code as u32 == FPDF_PAGEOBJ_FORM && depth < DEEPEST {
                let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
                unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
                let inner = matrices([m.a, m.b, m.c, m.d, m.e, m.f], to_page);
                let children = unsafe { bindings.FPDFFormObj_CountObjects(handle) };
                for i in (0..children).rev() {
                    let child = unsafe { bindings.FPDFFormObj_GetObject(handle, i as c_ulong) };
                    if !child.is_null() {
                        stack.push((child, depth + 1, top, inner));
                    }
                }
            }
        }

        if !text_page.is_null() {
            unsafe { bindings.FPDFText_ClosePage(text_page) };
        }

        // **A picture's opacity comes from the stream, not from PDFium.**
        // `FPDFPageObj_GetFillColor` has no answer for an image object, so the
        // `gs` in force at its `Do` is resolved through the page's ExtGState
        // resources to its `ca`. Read once for the page, only when it has a
        // picture at all — and not when only the paths are wanted.
        if !shapes_only && out.iter().any(|d| d.kind == DrawnKind::Picture && d.depth == 0) {
            if let Some(found) = self.picture_opacities(page_index) {
                for entry in out.iter_mut().filter(|d| d.kind == DrawnKind::Picture && d.depth == 0) {
                    if let Some(alpha) = found.get(&entry.object) {
                        entry.opacity = *alpha;
                    }
                }
            }
        }

        // **A placeholder is listed under its picture, not beside it.** The
        // grey rectangle a design program paints under every photograph is a
        // separate object to the page and one thing to anybody reading it.
        // Listed as its own row it doubled the length of the list and, picked
        // by mistake, was the "grey layer" that could not be moved. It stays
        // in the list — it is real, and it is where a hidden picture went —
        // but stepped in under the picture, which is what moves.
        let mut index = 0;
        while index + 1 < out.len() {
            let (a, b) = (&out[index], &out[index + 1]);
            let close = |x: f32, y: f32| (x - y).abs() <= 3.0;
            let placeholder = a.depth == 0
                && b.depth == 0
                && a.kind == DrawnKind::Shape
                && b.kind == DrawnKind::Picture
                && close(a.rect.left, b.rect.left)
                && close(a.rect.top, b.rect.top)
                && close(a.rect.right, b.rect.right)
                && close(a.rect.bottom, b.rect.bottom);
            if placeholder {
                out.swap(index, index + 1);
                let picture = out[index].object;
                let under = &mut out[index + 1];
                under.depth = 1;
                under.label = "placeholder".to_string();
                under.movable = false;
                // Addressed as its picture, so that picking it and moving it
                // moves the picture — the whole of which it is part.
                under.object = picture;
                index += 2;
            } else {
                index += 1;
            }
        }
        if shapes_only {
            out.retain(|d| d.kind == DrawnKind::Shape);
        }
        Ok(out)
    }

    fn stacking_neighbour(
        &self,
        page_index: usize,
        object: usize,
        up: bool,
    ) -> Result<Option<crate::document::DrawnObject>> {
        let drawn = self.drawn_objects(page_index)?;
        let top: Vec<&crate::document::DrawnObject> = drawn.iter().filter(|d| d.depth == 0).collect();
        let Some(me) = top.iter().find(|d| d.object == object) else {
            return Err(PdfError::InvalidArgument("that object is not on this page".into()));
        };
        let overlaps = |d: &crate::document::DrawnObject| {
            d.rect.left < me.rect.right
                && d.rect.right > me.rect.left
                && d.rect.top < me.rect.bottom
                && d.rect.bottom > me.rect.top
        };
        // In drawing order, so "the nearest above" is the first after it.
        let found = if up {
            top.iter().filter(|d| d.object > object).find(|d| overlaps(d))
        } else {
            top.iter().filter(|d| d.object < object).rev().find(|d| overlaps(d))
        };
        Ok(found.map(|d| (*d).clone()))
    }

    fn scale_object(
        &mut self,
        page_index: usize,
        object: usize,
        anchor: Point,
        sx: f32,
        sy: f32,
    ) -> Result<()> {
        if !(sx.is_finite() && sy.is_finite()) || sx.abs() < 1e-3 || sy.abs() < 1e-3 {
            return Err(PdfError::InvalidArgument("that would resize it to nothing".into()));
        }
        // Text resizes by font size and horizontal scale, not a geometric
        // `cm` — a shared text box refuses that (see `object_wrap_site`) and
        // it is the wrong lever for text anyway ("their size is a font
        // size"). `sx` and `sy` kept separate, not collapsed to one factor,
        // so a side handle stretches width alone and a top/bottom handle
        // changes size alone — a uniform factor made every handle resize
        // the same way regardless of which one was dragged. Byte-safe, like
        // the move path below: a `Tf`/`Tz` splice, not
        // `FPDFPage_GenerateContent` — that one needs a full before/after
        // text_runs() comparison to prove it did not scramble the rest of
        // the page (see `set_text_run_styled`), which on a page with a few
        // hundred runs was most of a resize's cost and, on some pages,
        // refused the resize outright because the regeneration really did
        // scramble something. This can't scramble anything to check for: it
        // touches only the operators it writes around this one run.
        if let Some(run) = self.text_run_at(page_index, object)? {
            let before = run.rect;
            self.resize_run_in_stream(page_index, &run, sx, sy)?;
            // Changing size grows the glyphs about the run's own baseline,
            // not about whichever corner the caller's handle anchored on —
            // a font's ascent so outweighs its descent that growing it
            // mostly pushes the *top* up, so a bottom handle dragged
            // downward, expecting the top to hold still, instead grew the
            // run upward. `resize_run_in_stream` already made it the right
            // *size*; this nudges it to where a geometric scale about
            // `anchor` (the branch below) would have left it, the same
            // move `move_run_in_stream` already knows how to make.
            if let Some(after) = self.text_run_at(page_index, object)? {
                let want_left = anchor.x + (before.left - anchor.x) * sx;
                let want_top = anchor.y + (before.top - anchor.y) * sy;
                let shift = Point { x: want_left - after.rect.left, y: want_top - after.rect.top };
                if shift.x.abs() > 0.01 || shift.y.abs() > 0.01 {
                    self.move_run_in_stream(page_index, object, shift)?;
                }
            }
            return Ok(());
        }
        // The anchor arrives top-left down; page space is bottom-left up.
        let height = self.page_size(page_index)?.height_pt;
        let (ax, ay) = (anchor.x, height - anchor.y);
        // Scale about the anchor: p' = (p − a)·S + a.
        let page_matrix = [sx, 0.0, 0.0, sy, ax * (1.0 - sx), ay * (1.0 - sy)];
        self.transform_in_stream(page_index, object, page_matrix)
    }

    fn rotate_object(
        &mut self,
        page_index: usize,
        object: usize,
        pivot: Point,
        degrees: f32,
    ) -> Result<()> {
        if !degrees.is_finite() || !pivot.x.is_finite() || !pivot.y.is_finite() {
            return Err(PdfError::InvalidArgument("that is not an angle".into()));
        }
        // Nothing to do, and nothing worth rewriting the page for.
        if degrees.rem_euclid(360.0).min(360.0 - degrees.rem_euclid(360.0)) < 1e-3 {
            return Ok(());
        }
        // The pivot arrives top-left down; page space is bottom-left up, where a
        // turn clockwise as seen is a negative angle.
        let height = self.page_size(page_index)?.height_pt;
        let (px, py) = (pivot.x, height - pivot.y);
        let (s, c) = (-degrees).to_radians().sin_cos();
        // p' = R(p − pivot) + pivot.
        let page_matrix = [c, s, -s, c, px - px * c + py * s, py - px * s - py * c];
        // Through the content stream — a matrix written round the operators that
        // draw this one thing, every other byte of the page copied as it is —
        // the same lever a resize and a move use for anything that is not words,
        // and for words too: a turn is the one change a text object's own
        // matrix is for. Words that share a text object with others cannot have
        // a matrix written round them, and go through PDFium's own object —
        // guarded, and refused with a reason if the page does not survive it.
        match self.transform_in_stream(page_index, object, page_matrix) {
            Err(PdfError::Unsupported(_)) if self.text_run_at(page_index, object)?.is_some() => {
                self.rotate_text_object_via_pdfium(page_index, object, page_matrix)
            }
            other => other,
        }
    }

    fn set_opacity(&mut self, page_index: usize, object: usize, opacity: f32) -> Result<()> {
        use crate::pdf::{content, Object};

        let opacity = opacity.clamp(0.0, 1.0);
        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let states = content::states(&operations);
        let placed = content::placed(&operations);
        let site = self.object_wrap_site(page_index, object, &file, &page, &operations, &states, &placed)?;

        // **A resource of its own on the page.** Opacity is a graphics-state
        // parameter set through an `ExtGState`, which has to be named in the
        // page's resources — written onto the page rather than into whatever
        // it inherits, because that is shared with every other page.
        let mut resources = self
            .inherited(&file, &page, b"Resources")?
            .and_then(|r| r.as_dict().cloned())
            .unwrap_or(crate::pdf::Dict(Vec::new()));
        let mut states_dict = resources
            .get(b"ExtGState")
            .and_then(|g| file.resolve(g).ok())
            .and_then(|g| g.as_dict().cloned())
            .unwrap_or(crate::pdf::Dict(Vec::new()));
        let mut name = b"PagifyAlpha1".to_vec();
        for suffix in 1..=999u32 {
            let candidate = format!("PagifyAlpha{suffix}").into_bytes();
            if states_dict.get(&candidate).is_none() {
                name = candidate;
                break;
            }
        }
        let number = file.next_object_number()?;
        let alpha = format!("{opacity:.4}");
        let alpha = alpha.trim_end_matches('0').trim_end_matches('.').to_string();
        let mut state = crate::pdf::Dict(Vec::new());
        state.set(b"Type", Object::Name(b"ExtGState".to_vec()));
        state.set(b"ca", Object::Number(alpha.clone().into_bytes()));
        state.set(b"CA", Object::Number(alpha.into_bytes()));
        let mut state_bytes = Vec::new();
        crate::pdf::write_object(&mut state_bytes, &Object::Dict(state));
        states_dict.set(&name, Object::Reference(number, 0));
        resources.set(b"ExtGState", Object::Dict(states_dict));
        let mut page_dict = page
            .as_dict()
            .cloned()
            .ok_or(PdfError::Unsupported("that page cannot be read"))?;
        page_dict.set(b"Resources", Object::Dict(resources));
        let page_number = self.page_object_number(&file, page_index)?;
        let mut page_bytes = Vec::new();
        crate::pdf::write_object(&mut page_bytes, &Object::Dict(page_dict));

        // **Absolute, not cumulative.** An opacity set earlier is a `gs` this
        // wrote just inside the object's scope; setting it again replaces that
        // operator rather than nesting another, or two settings of 50% would
        // draw at 25%.
        let gs = format!("/{} gs", String::from_utf8_lossy(&name)).into_bytes();
        let earlier = (site.span.start + 1..site.span.end).find(|i| {
            let op = &operations[*i];
            op.operator == b"gs"
                && matches!(op.operands.first(), Some(Object::Name(n)) if n.starts_with(b"PagifyAlpha"))
        });
        let edited = match (site.own_scope, earlier) {
            (_, Some(at)) => content::splice(&stream, &[(operations[at].span.clone(), gs)]),
            (true, None) => {
                let at = operations[site.span.start].span.end;
                content::splice(&stream, &[(at..at, gs)])
            }
            (false, None) => {
                let from = operations[site.span.start].span.start;
                let to = operations[site.span.end - 1].span.end;
                let mut opening: Vec<u8> = b"q\n".to_vec();
                opening.extend_from_slice(&gs);
                content::splice(&stream, &[(from..from, opening), (to..to, b"\nQ".to_vec())])
            }
        };

        let mut replacements = vec![(page_number, page_bytes)];
        for (index, (stream_number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*stream_number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[(number, state_bytes)])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    fn restack(
        &mut self,
        page_index: usize,
        object: usize,
        where_to: crate::document::Stacking,
    ) -> Result<()> {
        use crate::document::Stacking;
        use crate::pdf::content;

        let pictures = self.images_on(page_index)?;
        let runs = self.text_runs(page_index)?;
        let height = self.page_size(page_index)?.height_pt;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let states = content::states(&operations);
        let placed = content::placed(&operations);

        // **Which operations draw it, and whether they are words.** A run has
        // to be re-opened inside its own `BT`; a picture does not. A shape may
        // also set a clip — `W f`, one path doing two jobs — in which case the
        // clip stays where it is and only the paint travels.
        let mut self_clips = false;
        let (first, last, words) = if let Some(which) =
            pictures.iter().position(|i| i.object == object)
        {
            let names = self.image_names(&file, &page)?;
            let drawn = image_operators(&operations, &names);
            if drawn.len() != pictures.len() {
                return Err(PdfError::Unsupported(
                    "this page draws its pictures in a way this cannot follow",
                ));
            }
            // The whole unit — placeholder, frame and picture — see
            // `picture_unit`. Its own clip is inside the span and travels
            // as bytes, so it is not also replayed as a clip in force.
            let unit = picture_unit(&operations, drawn[which]);
            (unit.start, unit.end - 1, false)
        } else if let Some((first, last)) = self
            .object_spans(page_index, &file, &page, &operations, &placed)?
            .get(object)
            .copied()
            .flatten()
            .filter(|_| {
                // A group or a shading: one operator, placed by ordinal like a
                // picture. Words and shapes take their own branches below.
                use pdfium_render::prelude::{FPDF_PAGEOBJ_FORM, FPDF_PAGEOBJ_SHADING};
                self.object_census(page_index)
                    .ok()
                    .and_then(|c| c.get(object).map(|d| d.kind as u32))
                    .is_some_and(|k| k == FPDF_PAGEOBJ_FORM || k == FPDF_PAGEOBJ_SHADING)
            })
        {
            (first, last, false)
        } else if let Some((which, paths)) = self.path_ordinal(page_index, object)? {
            let painted = path_operators(&operations);
            if painted.len() != paths {
                return Err(PdfError::Unsupported(
                    "this page paints its shapes in a way this cannot follow",
                ));
            }
            let (span, clips) = painted
                .get(which)
                .cloned()
                .ok_or(PdfError::Unsupported("that shape is not painted on this page"))?;
            self_clips = clips;
            (span.start, span.end - 1, false)
        } else if let Some(run) = runs.iter().find(|r| r.object == object) {
            let fonts = self.page_fonts(&file, &page);
            let codes_in = |p: &content::Placed| -> usize {
                let width = p
                    .font
                    .as_ref()
                    .zip(fonts.as_ref())
                    .and_then(|(name, dict)| code_width(&file, dict, name))
                    .unwrap_or(1)
                    .max(1);
                content::pieces(&operations[p.origin.operation])
                    .iter()
                    .map(|piece| match piece {
                        content::Piece::Codes(bytes) => bytes.len() / width,
                        content::Piece::Kern(_) => 0,
                    })
                    .sum()
            };
            let order = self.text_order(page_index, object);
            let (first, last, continues) =
                run_operators(run, height, &placed, &operations, &codes_in, order)?;
            if continues {
                // Lifting them out takes their advance with them, and the words
                // that follow on the line would close up over the gap.
                return Err(PdfError::Unsupported(
                    "the words after these continue the same line",
                ));
            }
            (first, last, true)
        } else {
            return Err(PdfError::Unsupported(
                "this is drawn in a way this cannot follow through the drawing order",
            ));
        };

        // **"To the back" means behind the other things on the page, not
        // behind the paper.** A page usually opens with something that covers
        // all of it — a background panel, a gradient — and landing in front of
        // nothing at all puts the picture underneath that, where it cannot be
        // seen. Measured on a brochure: sent to the back, a photograph
        // vanished behind the page's gradient. So the back is just above the
        // last of the page-covering things the stream opens with.
        let background_end: Option<usize> = {
            let census = self.object_census(page_index)?;
            let spans = self.object_spans(page_index, &file, &page, &operations, &placed)?;
            let Some(me) = census.get(object).map(|d| d.rect) else {
                return Err(PdfError::InvalidArgument("that object is not on this page".into()));
            };
            // "Background" is relative: anything the page opens with that
            // would cover this object entirely. On a spread, the right page's
            // gradient is two-thirds of the sheet and all of any photograph
            // on it.
            let covers = |r: &Rect| {
                r.left <= me.left + 1.0
                    && r.top <= me.top + 1.0
                    && r.right >= me.right - 1.0
                    && r.bottom >= me.bottom - 1.0
            };
            // The nearest thing below this one that would hide it: the back
            // is just above that. On a spread the right page's gradient sits
            // in the middle of the stream, after the whole left page, so this
            // is a search from the object downwards, not from the top.
            (0..object)
                .rev()
                .filter(|i| spans.get(*i).copied().flatten() != spans.get(object).copied().flatten())
                .find(|i| census.get(*i).is_some_and(|d| covers(&d.rect)))
                .and_then(|i| spans.get(i).copied().flatten())
                .map(|(_, end)| end)
        };

        // Where it lands, and the transform in force there.
        let (landing_span, landing_ctm) = match where_to {
            // Just past the page's background, if it has one; otherwise
            // nothing has run yet at the head of the stream.
            Stacking::Back => match background_end {
                Some(after) => {
                    let mut after = after;
                    if inside_text_object(&operations, after) {
                        after = operations
                            .iter()
                            .enumerate()
                            .skip(after)
                            .find(|(_, op)| op.operator == b"ET")
                            .map(|(i, _)| i)
                            .unwrap_or(after);
                    }
                    after = leave_clips_after(&operations, &states, after);
                    (
                        operations[after].span.end..operations[after].span.end,
                        states[after].ctm,
                    )
                }
                None => (0..0, IDENTITY_MATRIX),
            },
            // Whatever is left in force once the page has finished drawing.
            Stacking::Front => (
                stream.len()..stream.len(),
                states.last().map(|s| s.ctm).unwrap_or(IDENTITY_MATRIX),
            ),
            // **One step: just past the neighbour.** The neighbour has to be
            // found in the stream too — whatever kind it is — and the landing
            // point moved out of any text object it sits in, because the block
            // this writes opens with `q`, which is not allowed inside one.
            Stacking::Up | Stacking::Down => {
                let spans = self.object_spans(page_index, &file, &page, &operations, &placed)?;
                // The next thing that is not part of this one's own unit — a
                // picture's placeholder shares its span, and stepping "down"
                // onto that would be stepping onto itself.
                let mine = spans.get(object).copied().flatten();
                let census = self.object_census(page_index)?;
                let me = census.get(object).map(|d| d.rect);
                // Stepping down never goes behind something that would hide
                // this object entirely — the step would look like it vanished.
                let hides = |n: usize| {
                    match (me, census.get(n)) {
                        (Some(me), Some(d)) => {
                            d.rect.left <= me.left + 1.0
                                && d.rect.top <= me.top + 1.0
                                && d.rect.right >= me.right - 1.0
                                && d.rect.bottom >= me.bottom - 1.0
                        }
                        _ => false,
                    }
                };
                // **The neighbour is the nearest thing that overlaps this
                // one**, not the next thing in the file — see
                // `stacking_neighbour`. Going down, one that would hide this
                // object entirely is passed over rather than gone behind.
                let neighbour = match self.stacking_neighbour(page_index, object, matches!(where_to, Stacking::Up))? {
                    Some(over) => match where_to {
                        Stacking::Up => Some(over.object),
                        _ => {
                            if hides(over.object) {
                                (0..over.object)
                                    .rev()
                                    .find(|n| (spans[*n] != mine || mine.is_none()) && !hides(*n)
                                        && census.get(*n).is_some_and(|d| {
                                            let me = me.expect("checked");
                                            d.rect.left < me.right && d.rect.right > me.left
                                                && d.rect.top < me.bottom && d.rect.bottom > me.top
                                        }))
                            } else {
                                Some(over.object)
                            }
                        }
                    },
                    None => None,
                };
                let Some(neighbour) = neighbour else {
                    return Err(PdfError::InvalidArgument(
                        match where_to {
                            Stacking::Up => "that is already in front of everything it overlaps",
                            _ => "that is already behind everything it overlaps",
                        }
                        .into(),
                    ));
                };
                let Some((n_first, n_last)) = spans[neighbour] else {
                    return Err(PdfError::Unsupported(
                        "the thing next to this is drawn in a way this cannot follow",
                    ));
                };
                // **And out of any clip the neighbour sits under.** The block
                // carries the object's own clips, but whatever clip is in
                // force where it lands applies on top — and a neighbour inside
                // a panel's clip would pull the object in under it, where it
                // may never have been and may not be visible at all.
                match where_to {
                    Stacking::Up => {
                        let mut after = n_last;
                        if inside_text_object(&operations, after) {
                            after = operations
                                .iter()
                                .enumerate()
                                .skip(after)
                                .find(|(_, op)| op.operator == b"ET")
                                .map(|(i, _)| i)
                                .unwrap_or(after);
                        }
                        after = leave_clips_after(&operations, &states, after);
                        (
                            operations[after].span.end..operations[after].span.end,
                            states[after].ctm,
                        )
                    }
                    _ => {
                        let mut before = n_first;
                        if inside_text_object(&operations, before) {
                            before = operations[..before]
                                .iter()
                                .rposition(|op| op.operator == b"BT")
                                .unwrap_or(before);
                        }
                        before = leave_clips_before(&operations, &states, before);
                        let ctm = before
                            .checked_sub(1)
                            .and_then(|i| states.get(i))
                            .map(|s| s.ctm)
                            .unwrap_or(IDENTITY_MATRIX);
                        (operations[before].span.start..operations[before].span.start, ctm)
                    }
                }
            }
        };

        let block = self.carried_block(
            &stream,
            &operations,
            &states,
            first,
            last,
            words,
            self_clips,
            landing_ctm,
        )?;

        // What stays behind. Ordinarily nothing; for a shape that also set a
        // clip, the same path closed with `n` — the clip, and none of the paint.
        let lifted = operations[first].span.start..operations[last].span.end;
        let stays = if self_clips {
            let mut kept: Vec<u8> = Vec::new();
            for operation in operations.iter().take(last + 1).skip(first) {
                if matches!(
                    operation.operator.as_slice(),
                    b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n"
                ) {
                    kept.extend_from_slice(b"n");
                } else {
                    kept.extend_from_slice(&stream[operation.span.clone()]);
                }
                kept.push(b'\n');
            }
            kept
        } else {
            Vec::new()
        };
        let edited = content::splice(&stream, &[(lifted, stays), (landing_span, block)]);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    fn object_bounds(&self, page_index: usize, object: usize) -> Result<Rect> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let space = raw.space()?;
        let bindings = pdfium()?.bindings();
        let index = i32::try_from(object).map_err(|_| {
            PdfError::InvalidArgument(format!("object {object} is out of range"))
        })?;
        let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
        if handle.is_null() {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has no object {object}",
                page_index + 1
            )));
        }
        let (mut l, mut b, mut r, mut t) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        if unsafe { bindings.FPDFPageObj_GetBounds(handle, &mut l, &mut b, &mut r, &mut t) } == 0 {
            return Err(PdfError::Pdfium("that object has no measurable bounds".into()));
        }
        let (left, top) = space.to_top_left(l, t);
        let (right, bottom) = space.to_top_left(r, b);
        Ok(Rect { left, top, right, bottom })
    }

    /// Same extraction `outlined_clusters`/`identify_outlined_glyphs` already
    /// do for font-recognition purposes (read every segment, flatten curves
    /// via `build_outline`) — reused here for one specific object rather than
    /// walked across the whole page, since a hit test only ever needs one
    /// candidate's ink at a time. The typed `pdfium-render` wrapper is used
    /// rather than `object_bounds`'s raw bindings above: `path.segments()`
    /// already transforms by the object's own matrix and classifies segment
    /// types, which a hand-rolled `FPDFPath_*` walk would have to redo.
    fn object_outline(&self, page_index: usize, object: usize) -> Result<Vec<Vec<(f32, f32)>>> {
        use crate::document::outlined::{flatten_segments, PathSegment, SegmentKind};
        use pdfium_render::prelude::{PdfPathSegmentType, PdfPathSegments};

        self.validate_page_index(page_index)?;
        let pdfium_index = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let page = self
            .document
            .pages()
            .get(pdfium_index)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let space = PageSpace::for_page(&page, page.height().value);

        let Some(page_object) = page.objects().iter().nth(object) else {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has no object {object}",
                page_index + 1
            )));
        };
        let Some(path) = page_object.as_path_object() else {
            return Err(PdfError::Unsupported("that object is not a path"));
        };
        let matrix = page_object.matrix().map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let mut segments: Vec<PathSegment> = Vec::new();
        for segment in path.segments().transform(matrix).iter() {
            let (x, y) = space.to_top_left(segment.x().value, segment.y().value);
            let kind = match segment.segment_type() {
                PdfPathSegmentType::MoveTo => SegmentKind::MoveTo,
                PdfPathSegmentType::LineTo => SegmentKind::LineTo,
                _ => SegmentKind::BezierTo,
            };
            segments.push(PathSegment { kind, x, y, close: segment.is_close() });
        }

        Ok(flatten_segments(&segments))
    }

    fn signature_marks(&self, page_index: usize) -> Result<Vec<crate::document::SignatureMark>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_GetAnnotCount(page.handle) };

        let mut found = Vec::new();
        for i in 0..count.max(0) {
            let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, i) };
            if annot.is_null() {
                continue;
            }
            // The key first: it is one string read, against reading every
            // stroke of every ink annotation on the page to find out it was a
            // drawing.
            let name = read_annotation_string(annot, SIGNATURE_KEY);
            let read = match &name {
                Some(_) => self.read_annotation(annot, &space),
                None => Ok(None),
            };
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };

            let (Some(name), Ok(Some(Annotation::Ink { strokes, color, width }))) = (name, read)
            else {
                continue;
            };
            found.push(crate::document::SignatureMark {
                // PDFium's index, which is what removes it — see `annotations`.
                index: i as usize,
                name,
                strokes,
                color,
                width,
            });
        }
        Ok(found)
    }

    fn image_signature_marks(
        &self,
        page_index: usize,
    ) -> Result<Vec<crate::document::ImageSignatureMark>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_GetAnnotCount(page.handle) };

        let mut found = Vec::new();
        for i in 0..count.max(0) {
            let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, i) };
            if annot.is_null() {
                continue;
            }
            let name = read_annotation_string(annot, SIGNATURE_KEY);
            let read = match &name {
                Some(_) => self.read_annotation(annot, &space),
                None => Ok(None),
            };
            // The id, if this picture kept one — read here, while the
            // annotation is still open, and resolved against `image_alpha`
            // below. See that field's doc for why the annotation's own
            // pixels (what `read` just produced) are a fallback, not the
            // first choice.
            let alpha_id = read_annotation_string(annot, ALPHA_ID_KEY).and_then(|s| s.parse::<u64>().ok());
            let rotation = read_rotation(annot);
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };

            let (Some(name), Ok(Some(Annotation::Image { rect, rgba, width, height }))) =
                (name, read)
            else {
                continue;
            };
            let resolved_alpha_id = alpha_id.filter(|id| self.image_alpha.contains_key(id));
            let rgba = resolved_alpha_id
                .and_then(|id| self.image_alpha.get(&id))
                .filter(|original| original.len() == rgba.len())
                .cloned()
                .unwrap_or(rgba);
            found.push(crate::document::ImageSignatureMark {
                index: i as usize,
                name,
                rect,
                rgba,
                width,
                height,
                alpha_id: resolved_alpha_id,
                rotation,
            });
        }
        Ok(found)
    }

    /// The complement of `image_signature_marks`: every image annotation
    /// that is *not* named as a signature. Two scans over the same
    /// annotation list rather than one split afterward, so the disjoint
    /// gate lives in one obvious place per function instead of a filter a
    /// caller could accidentally invert.
    fn placed_image_marks(
        &self,
        page_index: usize,
    ) -> Result<Vec<crate::document::PlacedImageMark>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_GetAnnotCount(page.handle) };

        let mut found = Vec::new();
        for i in 0..count.max(0) {
            let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, i) };
            if annot.is_null() {
                continue;
            }
            // A signature's own mark — leave it to `image_signature_marks`.
            if read_annotation_string(annot, SIGNATURE_KEY).is_some() {
                unsafe { bindings.FPDFPage_CloseAnnot(annot) };
                continue;
            }
            let read = self.read_annotation(annot, &space);
            let rotation = read_rotation(annot);
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };

            let Ok(Some(Annotation::Image { rect, rgba, width, height })) = read else {
                continue;
            };
            found.push(crate::document::PlacedImageMark {
                index: i as usize,
                rect,
                rgba,
                width,
                height,
                rotation,
            });
        }
        Ok(found)
    }

    fn annotation_count(&self, page_index: usize) -> Result<usize> {
        self.validate_page_index(page_index)?;
        let index = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;

        let page = RawPage::open(self.document.handle(), index)?;
        let count = unsafe { pdfium()?.bindings().FPDFPage_GetAnnotCount(page.handle) };
        Ok(count.max(0) as usize)
    }

    fn page_size(&self, index: usize) -> Result<PageSize> {
        self.validate_page_index(index)?;
        let pdfium_index = i32::try_from(index).map_err(|_| PdfError::PageOutOfRange {
            index,
            count: self.page_count,
        })?;

        let rect = self
            .document
            .pages()
            .page_size(pdfium_index)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        Ok(PageSize {
            width_pt: rect.width().value,
            height_pt: rect.height().value,
        })
    }

    fn page(&self, index: usize) -> Result<Box<dyn Page + '_>> {
        self.validate_page_index(index)?;
        let pdfium_index = i32::try_from(index).map_err(|_| PdfError::PageOutOfRange {
            index,
            count: self.page_count,
        })?;

        let page = self
            .document
            .pages()
            .get(pdfium_index)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        Ok(Box::new(PdfiumPage { page }))
    }
}

pub struct PdfiumPage<'a> {
    page: PdfPage<'a>,
}

impl<'a> Page for PdfiumPage<'a> {
    fn size(&self) -> PageSize {
        PageSize {
            width_pt: self.page.width().value,
            height_pt: self.page.height().value,
        }
    }

    fn render_into(&self, request: &RenderRequest, target: &mut RenderTarget<'_>) -> Result<()> {
        let config = build_render_config(request, target.width, target.height);

        if target.is_tightly_packed() {
            // Zero-copy: PDFium writes into the caller's buffer (an Android
            // bitmap's locked pixels on the on-screen path).
            {
                let mut pdf_bitmap = PdfBitmap::from_bytes(
                    target.width as i32,
                    target.height as i32,
                    PdfBitmapFormat::BGRA,
                    target.pixels,
                )
                .map_err(|e| PdfError::InvalidBitmap(e.to_string()))?;

                self.page
                    .render_into_bitmap_with_config(&mut pdf_bitmap, &config)
                    .map_err(|e| PdfError::Pdfium(e.to_string()))?;
            }
        } else {
            // The destination pads its rows and PDFium cannot be told about that,
            // so render tightly and blit row by row.
            let mut scratch =
                Bitmap::new(target.width, target.height, bitmap::PDFIUM_OUTPUT_ORDER)?;
            {
                let mut pdf_bitmap = PdfBitmap::from_bytes(
                    target.width as i32,
                    target.height as i32,
                    PdfBitmapFormat::BGRA,
                    &mut scratch.data,
                )
                .map_err(|e| PdfError::InvalidBitmap(e.to_string()))?;

                self.page
                    .render_into_bitmap_with_config(&mut pdf_bitmap, &config)
                    .map_err(|e| PdfError::Pdfium(e.to_string()))?;
            }
            // `copy_from` performs the BGRA -> target-order conversion itself.
            return target.copy_from(&scratch);
        }

        target.normalise_from_pdfium();
        Ok(())
    }

    fn render_region(&self, request: &RegionRequest) -> Result<Bitmap> {
        let region = RegionPixels::resolve(self.size(), request.crop, request.scale)?;

        let mut bitmap = Bitmap::new(region.width, region.height, bitmap::PDFIUM_OUTPUT_ORDER)?;
        // White, ourselves, rather than PDFium's `clear_before_rendering`. Its
        // clear fills from the *page's* origin, which for a crop sits off the
        // top-left of this bitmap — so the corner of the page that is not covered
        // by the page image would keep whatever the allocation held. Filling here
        // is unconditional and needs no reasoning about where the page landed.
        bitmap.data.fill(0xFF);

        let config = PdfRenderConfig::new()
            // The whole page, at the export scale...
            .set_target_size(region.page_width as i32, region.page_height as i32)
            // ...with its top-left pushed off this bitmap, so only the crop lands
            // inside it. PDFium clips to the destination, which is what makes the
            // guarantee that nothing outside the crop can appear structural
            // rather than a post-render trim.
            .set_origin(-region.offset_x, -region.offset_y)
            .set_format(PdfBitmapFormat::BGRA)
            .clear_before_rendering(false)
            .render_annotations(request.render_annotations)
            .render_form_data(request.render_form_data)
            .limit_render_image_cache_size(true);

        {
            let mut pdf_bitmap = PdfBitmap::from_bytes(
                region.width as i32,
                region.height as i32,
                PdfBitmapFormat::BGRA,
                &mut bitmap.data,
            )
            .map_err(|e| PdfError::InvalidBitmap(e.to_string()))?;

            self.page
                .render_into_bitmap_with_config(&mut pdf_bitmap, &config)
                .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        }

        Ok(bitmap)
    }

    fn text(&self) -> Result<String> {
        Ok(self
            .page
            .text()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?
            .all())
    }

    fn text_segments(&self) -> Result<Vec<TextSegment>> {
        // The crop, not the page height. PDFium reports and renders the CropBox
        // but hands back text geometry in MediaBox space, so on a page whose crop
        // is inset the two differ by exactly that inset — see `PageSpace`.
        let space = PageSpace::for_page(&self.page, self.page.height().value);
        let text = self
            .page
            .text()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let mut segments = Vec::new();
        for segment in text.segments().iter() {
            let content = segment.text();
            // Whitespace-only runs carry no glyphs to highlight and would only
            // add gaps to a selection.
            if content.trim().is_empty() {
                continue;
            }

            let bounds = segment.bounds();

            // PDF space puts the origin at the bottom-left with y increasing
            // upwards; every consumer of this wants the crop's top-left with y
            // increasing down. Converting once, here, keeps it out of the UI.
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);

            // Size and font come from the run's first character. Colour and
            // object identity are *not* here: PDFium exposes neither per
            // character nor per run at chromium/7881, and a field that is
            // always empty is worse than one that does not exist.
            let (font_size, font_name) = match segment.chars() {
                Ok(chars) => chars
                    .iter()
                    .next()
                    .map(|c| (c.scaled_font_size().value, c.font_name()))
                    .unwrap_or((0.0, String::new())),
                Err(_) => (0.0, String::new()),
            };

            segments.push(TextSegment {
                left,
                top,
                right,
                bottom,
                direction: crate::document::layout::Direction::of(&content),
                text: content,
                font_size,
                font_name,
            });
        }
        Ok(segments)
    }

    fn glyphs(&self) -> Result<Vec<crate::document::layout::Glyph>> {
        use crate::document::layout::{Glyph, Rect as LayoutRect};

        // The crop, not the page height — PDFium reports text geometry in
        // MediaBox space while rendering from the CropBox, so on an inset page
        // the two differ by exactly that inset. Every other reader of this
        // geometry goes through the same conversion.
        let space = PageSpace::for_page(&self.page, self.page.height().value);
        let text = self.page.text().map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let mut glyphs = Vec::new();
        for character in text.chars().iter() {
            let Some(ch) = character.unicode_char() else { continue };
            if ch == '\0' {
                // Unmappable. Carrying it as NUL would put a phantom character
                // into every reconstructed line.
                continue;
            }

            let bounds = match character.loose_bounds() {
                Ok(bounds) => bounds,
                Err(_) => continue,
            };
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);

            glyphs.push(Glyph {
                ch,
                rect: LayoutRect { left, top, right, bottom },
                angle: character.angle_radians().unwrap_or(0.0),
            });
        }
        Ok(glyphs)
    }

    /// Read type converted to outlines by matching its paths against a
    /// candidate face's own glyphs.
    ///
    /// **Filtered by the same shape test redaction uses**, before any curve is
    /// flattened or any match attempted: [`crate::document::classify::looks_like_type`]
    /// keeps a table rule, a logo, a signature stroke out of the pipeline
    /// entirely, rather than letting the matcher shrug them off one at a time.
    /// The two features cannot disagree about which paths are letters, because
    /// they ask the same function.
    fn recognise_outlined(&self, catalogue: &crate::document::glyphs::Catalogue) -> Result<crate::document::layout::PageText> {
        let identified = identify_outlined_glyphs(&self.page, catalogue);
        let glyphs = crate::document::outlined::as_glyphs(&identified);
        Ok(crate::document::layout::reconstruct(&glyphs))
    }

    fn recognise_outlined_words(
        &self,
        catalogue: &crate::document::glyphs::Catalogue,
    ) -> Result<Vec<crate::document::RecognisedWord>> {
        let identified = identify_outlined_glyphs(&self.page, catalogue);
        Ok(crate::document::outlined::as_words(&identified, OUTLINE_MATCH_TOLERANCE))
    }

    fn classify(&self) -> Result<PageClassification> {
        // Everything here goes through pdfium-render's safe wrappers rather
        // than raw bindings, which is worth a note because the plan reaches for
        // `FPDFText_HasUnicodeMapError`.
        //
        // That function is marked *Experimental API* in `fpdf_text.h`, it can
        // change between PDFium builds, and reaching it needs an
        // `FPDF_TEXTPAGE` handle that pdfium-render keeps `pub(crate)` — so it
        // would cost a second change to the vendored fork on top of an API with
        // no stability promise.
        //
        // The signal it would give is available without either. PDFium reports
        // a character it could not map as Unicode **0**, which is what
        // `unicode_value()` returns here. `examples/classify_probe.rs` measures
        // the two against each other on real files; if the zero signal is ever
        // shown to miss cases the explicit API catches, that is the moment to
        // pay for the fork change, and not before.
        let mut chars = 0usize;
        let mut unmappable = 0usize;
        let mut rotated_chars = 0usize;

        if let Ok(text) = self.page.text() {
            for character in text.chars().iter() {
                chars += 1;
                // Zero **or** U+FFFD. The zero-only test was written on the
                // reasoning that PDFium reports an unmapped glyph as Unicode
                // zero, with a note to widen it if a real file ever showed
                // otherwise. One did: an `fi` ligature in a real report comes
                // back as U+FFFD, and the page was classified `Native` with
                // zero unmappable characters while a word was silently broken.
                //
                // Still no fork change needed for it — the explicit
                // `FPDFText_HasUnicodeMapError` API would be a second patch to
                // a vendored dependency, and this catches the case for free.
                if matches!(character.unicode_value(), 0 | 0xFFFD) {
                    unmappable += 1;
                }
                if let Ok(angle) = character.angle_radians() {
                    if angle.abs() > 0.01 {
                        rotated_chars += 1;
                    }
                }
            }
        }

        // Page area from the crop box, because that is what a reader sees and
        // what "covers most of the page" has to mean.
        let page_area = (self.page.width().value * self.page.height().value).max(1.0);
        let mut image_area = 0.0f32;
        let mut paths = 0usize;
        let mut glyph_paths = 0usize;

        for object in self.page.objects().iter() {
            let bounds = match object.bounds() {
                Ok(bounds) => bounds,
                Err(_) => continue,
            };
            let width = (bounds.right().value - bounds.left().value).abs();
            let height = (bounds.top().value - bounds.bottom().value).abs();

            match object.object_type() {
                PdfPageObjectType::Image => image_area += width * height,
                PdfPageObjectType::Path => {
                    paths += 1;

                    let segments = object
                        .as_path_object()
                        .map(|p| {
                            use pdfium_render::prelude::PdfPathSegments;
                            p.segments().len() as usize
                        })
                        .unwrap_or(0);

                    // The same rule redaction uses, so the two cannot disagree
                    // about whether a page is letters or decoration.
                    if crate::document::classify::looks_like_type(
                        width,
                        height,
                        segments,
                        self.page.width().value,
                        self.page.height().value,
                    ) {
                        glyph_paths += 1;
                    }
                }
                _ => {}
            }
        }

        // Clamped, because overlapping images would otherwise report more than
        // a whole page and make the fraction meaningless.
        let image_coverage = (image_area / page_area).clamp(0.0, 1.0);

        let mut classification = PageClassification {
            kind: PageTextKind::Empty,
            chars,
            unmappable,
            image_coverage,
            paths,
            glyph_paths,
            rotated_chars,
        };
        classification.kind = classification.verdict();
        Ok(classification)
    }

    fn characters(&self) -> Result<PageCharacters> {
        // The crop again, for the same reason as the runs: PDFium reports text
        // geometry in MediaBox space while rendering from the CropBox.
        let space = PageSpace::for_page(&self.page, self.page.height().value);
        let text = self
            .page
            .text()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let characters = text.chars();
        let expected = characters.len() as usize;
        let mut out = PageCharacters {
            text: String::with_capacity(expected),
            boxes: Vec::with_capacity(expected * 4),
        };

        for character in characters.iter() {
            let Some(glyph) = character.unicode_char() else {
                // A character PDFium cannot map to Unicode has nothing to copy
                // and nothing to point at. Skipping it keeps the text and the
                // boxes aligned, which is the only thing this type promises.
                continue;
            };

            // Loose bounds rather than tight: a selection should cover the line's
            // full height, so consecutive characters join into an unbroken band
            // rather than a row of glyph-shaped bites.
            let Ok(bounds) = character.loose_bounds() else {
                continue;
            };
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) = space.to_top_left(bounds.right().value, bounds.bottom().value);

            out.text.push(glyph);
            // Once per UTF-16 code unit, so a character outside the basic plane
            // does not slide every box after it by one on the Kotlin side.
            for _ in 0..glyph.len_utf16() {
                out.boxes.extend_from_slice(&[left, top, right, bottom]);
            }
        }

        Ok(out)
    }
}

impl PdfiumDocument {
    /// Every text object on the page, including ones with no visible ink —
    /// unlike [`Document::text_runs`], which drops those because there is
    /// nothing there to click.
    ///
    /// A run blanked down to a single space (see `apply_paragraph_edit` in
    /// the app, which does this to lines a shrinking paragraph no longer
    /// needs) has a space glyph's empty ink outline, so it fails that
    /// click-target filter — and a lookup meant to find an *already-known*
    /// object by its own id has nothing to do with what a click could land
    /// on. Reported from use: undoing exactly that blank came back "that is
    /// not a text run," because the very run being reverted had vanished
    /// from the filtered list before its own revert could look it up.
    ///
    /// An inherent method, not part of the `Document` trait — it exists
    /// purely so `set_text_run_styled`, `set_run_in_stream`, and
    /// `set_run_color_in_stream` (all in `DocumentMut`'s impl, or below it)
    /// can look up a run by id without the click filter, alongside
    /// `text_runs` itself, which is in `Document`'s impl and needs the same
    /// lookup to build its own filtered list.
    fn text_runs_all(&self, page_index: usize) -> Result<Vec<crate::document::TextRun>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = self
            .document
            .pages()
            .get(page_number)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        // **The origins first, and the handle closed before the walk begins.**
        //
        // The matrix that says where a run is drawn from is only reachable
        // through a raw page handle, and this function already holds one of
        // PDFium's through `pages().get`. Two live handles on one page is the
        // arrangement this file warns about elsewhere in as many words — see
        // `set_text_run_styled` — so they are not held at once: everything the
        // raw handle knows is read here and it is dropped.
        let (space, origins, mut read_in_one_pass) = {
            let raw = RawPage::open(self.document.handle(), page_number)?;
            let space = raw.space()?;
            let bindings = pdfium()?.bindings();
            let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
            let mut origins = Vec::with_capacity(count.max(0) as usize);
            let mut text_objects: Vec<(usize, FPDF_PAGEOBJECT)> = Vec::new();
            for index in 0..count.max(0) {
                let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
                let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
                let read = !handle.is_null()
                    && unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) } != 0;
                origins.push(read.then(|| space.to_top_left(m.e, m.f)));
                if !handle.is_null()
                    && unsafe { bindings.FPDFPageObj_GetType(handle) } == FPDF_PAGEOBJ_TEXT as i32
                {
                    text_objects.push((index as usize, handle));
                }
            }
            // **A large page's words are read in one pass over its characters,
            // not one pass per object** — see [`LINEAR_TEXT_READ_FROM`]. `None`
            // for a page that is not large, and for one whose characters could
            // not be listed, which fall back to the per-object read below.
            let words = (text_objects.len() > LINEAR_TEXT_READ_FROM)
                .then(|| words_by_characters(bindings, raw.handle, &text_objects))
                .flatten();
            (space, origins, words)
        };
        // The text page is what turns a run's bytes into characters — the same
        // machinery extraction uses, so a run reads the way the rest of the
        // program reads the page. Not loaded when the words are already read.
        let text_page = match read_in_one_pass {
            Some(_) => None,
            None => Some(page.text().map_err(|e| PdfError::Pdfium(e.to_string()))?),
        };

        let mut runs = Vec::new();
        for (index, object) in page.objects().iter().enumerate() {
            let Some(text_object) = object.as_text_object() else { continue };
            // **Read through the text layer loaded above, once for the page.**
            // `text_object.text()` loads a text layer of its own on every
            // call — and, for an object with no words, returns without
            // closing it: seconds of work on a page of a thousand runs, and a
            // handle left open per empty one. Measured on a three-page
            // datasheet, the words of every text object took 1.1, 2.4 and
            // 2.4 s per page that way and 24, 36 and 39 ms this way, every
            // string identical (see `tests/text_words.rs`, which keeps the old
            // call as its oracle and the numbers in `what_the_words_cost`).
            //
            // **On a large page, already read** — see `read_in_one_pass` above.
            let words = match (&mut read_in_one_pass, &text_page) {
                (Some(read), _) => read.remove(&index).unwrap_or_default(),
                (None, Some(text_page)) => text_page.for_object(text_object),
                (None, None) => String::new(),
            };
            // Kept even when it reports no words.
            //
            // A run whose font has no `/ToUnicode` reads as empty here while
            // being perfectly visible on the page — which is most of "some
            // words are not recognised". Dropping those made them unclickable,
            // and a word you can see but cannot point at is worse than one
            // labelled unreadable.
            //
            // A run with no *area* is a different thing, and `text_runs()`
            // (above) drops it: it draws nothing, so there is nothing to
            // have clicked on. That filter is applied there, not here, so an
            // identity lookup by object id can still find a run this thin.
            let Ok(bounds) = object.bounds() else { continue };
            let (left, top) = space.to_top_left(bounds.left().value, bounds.top().value);
            let (right, bottom) =
                space.to_top_left(bounds.right().value, bounds.bottom().value);

            // The colour it is actually drawn in, so an editor can show it and
            // an edit can put it back.
            let colour = object
                .fill_color()
                .map(|c| Color { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() })
                .unwrap_or(Color { r: 0, g: 0, b: 0, a: 255 });

            // Where the text is drawn from, which is the matrix's translation
            // — the baseline, not the top of the box above it.
            let origin = match origins.get(index).copied().flatten() {
                Some((x, y)) => Point { x, y },
                // Better a point on the run than no run at all; the box's
                // bottom-left is the closest thing to a baseline there is.
                None => Point { x: left, y: bottom },
            };

            runs.push(crate::document::TextRun {
                object: index,
                text: words,
                rect: Rect { left, top, right, bottom },
                origin,
                // The *effective* size, folding in whatever vertical stretch
                // the text matrix carries — see `set_text_run_styled`'s own
                // `vertical_scale` doc for the producer that makes this
                // matter: `1 Tf` with the real size baked into the matrix,
                // which `unscaled_font_size` alone would report as "1pt".
                size: text_object.scaled_font_size().value,
                color: colour,
            });
        }
        drop(text_page);
        Ok(runs)
    }

    /// The words of every text object of a page, read in one pass over the page's
    /// characters whatever its size: what [`Document::text_runs_unfiltered`]
    /// puts in each run's `text` for a page above [`LINEAR_TEXT_READ_FROM`]
    /// objects, here for any page — chiefly so a test can hold it to PDFium's
    /// own per-object call (`tests/page_weight.rs` does, on the datasheet, the
    /// repository's fixtures and a page built to be awkward, and on any documents
    /// it is pointed at). An object with no words is in the map with an empty
    /// string.
    pub fn text_words_by_characters(&self, page_index: usize) -> Result<HashMap<usize, String>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
        let mut text_objects: Vec<(usize, FPDF_PAGEOBJECT)> = Vec::new();
        for index in 0..count.max(0) {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if !handle.is_null() && unsafe { bindings.FPDFPageObj_GetType(handle) } == FPDF_PAGEOBJ_TEXT as i32 {
                text_objects.push((index as usize, handle));
            }
        }
        let mut words = words_by_characters(bindings, raw.handle, &text_objects)
            .ok_or_else(|| PdfError::Pdfium("the page's characters could not be listed".into()))?;
        for (index, _) in &text_objects {
            words.entry(*index).or_default();
        }
        Ok(words)
    }
}

/// Pages with more text objects than this have their words read in one pass
/// over the page's characters ([`words_by_characters`]); a page with this many
/// or fewer reads them one text object at a time, through PDFium's own call.
///
/// **Why there are two ways.** `FPDFTextObj_GetText` finds an object's characters
/// by walking *every* character of the page, so reading the words of all *n*
/// objects is *n* walks of *n* objects' characters — **quadratic**. Measured
/// (this PDFium, one thread, a page of one word per object): 1,000 objects 30 ms,
/// 5,000 0.75 s, 10,000 3.3 s, 40,000 about 140 s (136 to 148 over three runs).
/// One pass over the characters, giving each to the object that drew it, is
/// linear — the same pages in 4 ms, 21 ms, 44 ms and 217 ms (50,000: 0.27 s) —
/// and gives the same words: character for
/// character, on all 28,692 text objects of the first four pages of 105 documents
/// (the owner's own quotations, invoices, receipts, datasheets and plots among
/// them), and on every page of the datasheet and of the repository's fixtures,
/// which `tests/page_weight.rs` holds it to.
///
/// **Why not always.** PDFium's own call is the definition of what an object
/// says, and a page small enough for its cost not to matter stays with it: the
/// pass over the characters re-implements a rule (below) that is verified, not
/// proved. At 1,500 objects the per-object read costs about 66 ms; the datasheet's
/// pages (883, 1,488 and 1,394 objects) stay on PDFium's own call.
const LINEAR_TEXT_READ_FROM: usize = 1500;

/// Below this, in PDF points, a text object's bounding box is treated as
/// having no area at all — the "nothing to have clicked on" rule
/// `text_runs()`, `text_run_rects()` and `text_run_at()` each apply to leave
/// out truly degenerate objects.
///
/// **Not a legibility cutoff.** This used to be `0.5`, chosen generously
/// rather than measured, and it silently dropped real, readable glyphs along
/// with the genuinely empty objects it was meant for: a capital "I" in a
/// condensed 8pt font measured at 0.41pt wide — real ink, a real character —
/// and `text_runs()`'s own 0.5pt floor excluded it from every pasted copy of
/// that font's text that put each glyph in its own object (`write_text`'s
/// "one text object per glyph"). Reported from use as "the pasted text isn't
/// what I copied" — the word was right there on the page, just unreadable to
/// Pagify's own text model. Set low enough to still catch what is actually
/// zero, or close enough to it to be pixel-rounding noise, and nothing a real
/// glyph could ever measure.
const HAS_AREA_PT: f32 = 0.05;

/// Whether a text object's bounds are wide and tall enough to mean it drew
/// something — see [`HAS_AREA_PT`].
fn has_area(left: f32, top: f32, right: f32, bottom: f32) -> bool {
    (right - left).abs() > HAS_AREA_PT && (top - bottom).abs() > HAS_AREA_PT
}

/// The words of every text object of an open page, in one pass over the page's
/// characters: the same strings `FPDFTextObj_GetText` gives for each, without
/// walking every character of the page once per object. `text_objects` are the
/// page's text objects, as (index in the page's object list, handle). `None`
/// when the page's characters cannot be listed.
///
/// **The rule being reproduced** (found by reading what the call answers, then
/// held to it on 105 documents): the words of an object are its own characters in
/// the order the page lists them, **less U+0000** — and **one space after them when
/// the character the page lists straight after them is a space** (whoever drew
/// it: a space of the object next to it, or one PDFium made up between two words),
/// a space that is how PDFium reports that a run was followed by one. An object
/// with no characters says nothing. A space follows a character of an object
/// only through the character *directly* after it: one object's character
/// between them and it is not appended. A string that is not valid UTF-16 reads
/// as empty, which is what the safe wrapper does with it.
///
/// Characters come as UTF-16 code units on Windows and as scalar values
/// elsewhere; both are put back into UTF-16 before the string is made.
fn words_by_characters(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    text_objects: &[(usize, FPDF_PAGEOBJECT)],
) -> Option<HashMap<usize, String>> {
    let text_page = unsafe { bindings.FPDFText_LoadPage(page) };
    if text_page.is_null() {
        return None;
    }
    let owner_of: HashMap<usize, usize> =
        text_objects.iter().map(|(index, handle)| (*handle as usize, *index)).collect();
    let characters = unsafe { bindings.FPDFText_CountChars(text_page) }.max(0);

    let mut units: HashMap<usize, Vec<u16>> = HashMap::with_capacity(text_objects.len());
    let push = |units: &mut HashMap<usize, Vec<u16>>, object: usize, unicode: u32| {
        // U+0000 is never part of an object's words.
        if unicode == 0 {
            return;
        }
        let list = units.entry(object).or_default();
        match u16::try_from(unicode) {
            Ok(unit) => list.push(unit),
            Err(_) => {
                if let Some(c) = char::from_u32(unicode) {
                    let mut pair = [0u16; 2];
                    list.extend_from_slice(c.encode_utf16(&mut pair));
                }
            }
        }
    };

    // The object that drew the previous character: the one a space can still be
    // appended to. The handle of the last character looked up, and what it was.
    let mut previous: Option<usize> = None;
    let mut last: (FPDF_PAGEOBJECT, Option<usize>) = (std::ptr::null_mut(), None);
    for at in 0..characters {
        let unicode = unsafe { bindings.FPDFText_GetUnicode(text_page, at) };
        let handle = unsafe { bindings.FPDFText_GetTextObject(text_page, at) };
        let owner = if handle.is_null() {
            None
        } else if handle == last.0 {
            last.1
        } else {
            let found = owner_of.get(&(handle as usize)).copied();
            last = (handle, found);
            found
        };
        if owner != previous {
            // A character that is not the previous object's own: if it is a space
            // it is the one that follows that object's last character.
            if let (Some(p), true) = (previous, unicode == 0x20) {
                push(&mut units, p, 0x20);
            }
        }
        if let Some(o) = owner {
            push(&mut units, o, unicode);
        }
        previous = owner;
    }
    unsafe { bindings.FPDFText_ClosePage(text_page) };

    Some(units.into_iter().map(|(object, units)| (object, String::from_utf16(&units).unwrap_or_default())).collect())
}

fn build_render_config(request: &RenderRequest, width: u32, height: u32) -> PdfRenderConfig {
    let rotation = match request.rotation {
        Rotation::None => PdfPageRenderRotation::None,
        Rotation::Clockwise90 => PdfPageRenderRotation::Degrees90,
        Rotation::Clockwise180 => PdfPageRenderRotation::Degrees180,
        Rotation::Clockwise270 => PdfPageRenderRotation::Degrees270,
    };

    PdfRenderConfig::new()
        .set_target_size(width as i32, height as i32)
        .set_format(PdfBitmapFormat::BGRA)
        .rotate(rotation, false)
        // PDFium leaves the page transparent unless told otherwise, which would
        // show as black once composited into an opaque Android bitmap.
        .clear_before_rendering(true)
        .set_clear_color(PdfColor::WHITE)
        .render_annotations(request.render_annotations)
        .render_form_data(request.render_form_data)
        // Bounds PDFium's cache of decoded images. Without it, a document whose
        // pages carry tens of megabytes of imagery each (a 2.9 GB catalogue works
        // out at ~31 MB per page) accumulates decoded bitmaps far larger than
        // anything being drawn — the cost lands on a small thumbnail render just
        // as hard as on a full page, because the decode is the same either way.
        .limit_render_image_cache_size(true)
}

/// A PDF hex string for user-facing text such as a bookmark's title: UTF-16BE
/// with the byte-order mark real readers look for before they will render
/// anything past plain ASCII, hex-digit-encoded because that is the form
/// [`crate::pdf::write_object`] writes an [`crate::pdf::Object::HexString`]'s
/// bytes back out **verbatim** between the angle brackets — see that
/// function's own handling of the variant. A `LiteralString` would need its
/// own escaping for parentheses and backslashes; a title typed by hand is
/// exactly where those show up.
fn utf16be_hex(text: &str) -> Vec<u8> {
    let mut units = vec![0xFEu8, 0xFF];
    for unit in text.encode_utf16() {
        units.extend_from_slice(&unit.to_be_bytes());
    }
    let mut hex = Vec::with_capacity(units.len() * 2);
    for byte in units {
        hex.extend_from_slice(format!("{byte:02X}").as_bytes());
    }
    hex
}

/// The page-tree half of the write path.
///
/// Three operations are missing, and all three fail for the *same* reason rather
/// than three: `pdfium-render` 0.9.3 keeps `PdfDocument::handle()` `pub(crate)`,
/// so the raw `FPDF_DOCUMENT` cannot be reached from outside the crate. PDFium
/// itself does all three perfectly well — `examples/incremental_probe.rs`
/// measures the save working through the raw bindings — and the functions are on
/// the public bindings trait. Only the handle is out of reach.
///
/// So the fix is one line upstream, not a change of engine, and making the
/// handle public unblocks page deletion, reordering and incremental save
/// together. Adding a save variant alone would not.
impl DocumentMut for PdfiumDocument {
    fn import_pages(
        &mut self,
        source: &dyn Document,
        indices: &[usize],
        at: usize,
    ) -> Result<usize> {
        let handle = source.backend_handle().ok_or_else(|| {
            PdfError::InvalidArgument("that document is not one PDFium opened".into())
        })?;
        // Safety: the value came from `backend_handle` on a live document the
        // caller is holding for the duration of this call, and only this
        // implementation ever produces or reads it.
        self.import_pages_from(handle as FPDF_DOCUMENT, indices, at)
    }

    fn insert_blank_page(
        &mut self,
        at: usize,
        size: PageSize,
        fill: Option<Color>,
        ruling: Ruling,
    ) -> Result<()> {
        let index = i32::try_from(at)
            .map_err(|_| PdfError::InvalidArgument(format!("page index {at} is out of range")))?;

        self.document
            .pages_mut()
            .create_page_at_index(
                PdfPagePaperSize::from_points(
                    PdfPoints::new(size.width_pt),
                    PdfPoints::new(size.height_pt),
                ),
                index,
            )
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        self.page_count += 1;
        self.touch();

        let page = RawPage::open(self.document.handle(), index)?;
        let bindings = pdfium()?.bindings();

        // A page has no colour of its own: white is what an empty one looks like.
        // A coloured sheet is therefore a rectangle covering it, filled and
        // written into the page's content — so it prints, and it is still there
        // when the file is opened anywhere else.
        // Safety: the page is live for the block and closed by RawPage's drop.
        unsafe {
            if let Some(paint) = fill {
                let rect = bindings.FPDFPageObj_CreateNewRect(
                    0.0,
                    0.0,
                    size.width_pt,
                    size.height_pt,
                );
                if rect.is_null() {
                    return Err(PdfError::Pdfium("could not create the sheet".into()));
                }
                bindings.FPDFPageObj_SetFillColor(
                    rect,
                    paint.r as c_uint,
                    paint.g as c_uint,
                    paint.b as c_uint,
                    paint.a as c_uint,
                );
                // Filled, not stroked: an outline round the edge of the sheet is
                // a border, which is not what was asked for.
                bindings.FPDFPath_SetDrawMode(rect, 1, 0);
                bindings.FPDFPage_InsertObject(page.handle, rect);
            }

            // The same ruling a whole new document gets, so a sheet added to a
            // notebook matches the sheets already in it.
            crate::document::blank::rule_page(
                bindings,
                page.handle,
                size,
                ruling,
                crate::document::blank::ruling_ink(
                    fill.unwrap_or(Color { r: 255, g: 255, b: 255, a: 255 }),
                ),
            )?;

            // Once, after everything: generating content per object rewrites the
            // stream each time, and skipping it entirely leaves a page whose
            // objects exist but are not drawn.
            if (fill.is_some() || ruling != Ruling::None)
                && bindings.FPDFPage_GenerateContent(page.handle) == 0
            {
                return Err(PdfError::Pdfium(
                    "the sheet was made but its content was not written".into(),
                ));
            }
        }

        Ok(())
    }

    fn set_page_rotation(&mut self, index: usize, quarter_turns: u8) -> Result<()> {
        self.validate_page_index(index)?;
        let page_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;

        let mut page = self
            .document
            .pages()
            .get(page_index)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        page.set_rotation(match quarter_turns % 4 {
            1 => PdfPageRenderRotation::Degrees90,
            2 => PdfPageRenderRotation::Degrees180,
            3 => PdfPageRenderRotation::Degrees270,
            _ => PdfPageRenderRotation::None,
        });

        self.touch();
        Ok(())
    }

    /// The rotation a page is currently at, in quarter turns.
    ///
    /// Read *before* a change so undo can put it back; without this the undo
    /// record could only ever restore zero, which is right exactly when the page
    /// was unrotated to begin with.
    fn page_crop(&self, index: usize) -> Result<Rect> {
        self.validate_page_index(index)?;
        let page_number = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;
        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();

        let (mut left, mut bottom, mut right, mut top) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // Falls back to the media box, which is what PDFium itself does when a
        // page declares no crop — most pages do not.
        let read = unsafe {
            bindings.FPDFPage_GetCropBox(page.handle, &mut left, &mut bottom, &mut right, &mut top)
        } != 0
            || unsafe {
                bindings.FPDFPage_GetMediaBox(
                    page.handle,
                    &mut left,
                    &mut bottom,
                    &mut right,
                    &mut top,
                )
            } != 0;
        if !read {
            return Err(PdfError::Pdfium("this page declares no boundaries".into()));
        }

        // PDF space is bottom-left; every rectangle this crate hands out is
        // top-left.
        let (l, t) = space.to_top_left(left, top);
        let (r, b) = space.to_top_left(right, bottom);
        Ok(Rect { left: l, top: t, right: r, bottom: b })
    }

    fn set_text_run_styled(
        &mut self,
        page_index: usize,
        object: usize,
        text: &str,
        style: &crate::document::TextStyle,
    ) -> Result<(String, crate::document::TextStyle)> {
        // **Change the words in the content stream if it can be done there.**
        //
        // Reported from use: editing a word changed the text around it.
        // `FPDFText_SetText` needs `FPDFPage_GenerateContent`, which re-emits
        // the whole page — the same thing that was rewriting paragraphs when a
        // phrase was locked. Swapping the codes in place touches nothing else.
        //
        // Only when a colour or a position is what's changing does this fall
        // through to PDFium: those still need it. A requested font is a
        // byte-safe swap of the same shape as the words themselves (see
        // `Self::set_run_in_stream`), and a new size is the same `Tf` that
        // already has to be written in front of this run's own text to
        // select its font — changing the number in it costs nothing extra —
        // so both travel with this fast path rather than gating it, checked
        // with `face`/`size` zeroed out on both sides so neither falls
        // through to the slower path below on its own.
        //
        // **Reported from use: a run's size could not be increased at all**
        // on a page where PDFium's whole-page regeneration corrupts other
        // text — the same defect words-only editing was already routed
        // around. Sizing a run is no different a change to make than
        // retyping it: both rewrite the one `Tf [...] TJ` this run already
        // draws with and nothing else in the stream.
        // **A colour-only change, retyping nothing, is narrower still.** The
        // codes drawing this run do not change at all, so nothing needs
        // matching against a font's encoding — checked first because it is
        // strictly safer than the fast path below, which still has to prove
        // the (possibly new) words can be spelled in *some* font. A colour
        // change alongside a retype falls through to the checks below
        // unchanged, exactly as it already did.
        if style.face.is_none() && style.size.is_none() && style.at.is_none() {
            if let Some(color) = style.color {
                let current = self
                    .text_runs_all(page_index)?
                    .into_iter()
                    .find(|r| r.object == object)
                    .ok_or_else(|| PdfError::InvalidArgument("that is not a text run".into()))?;
                if current.text == text {
                    return match self.set_run_color_in_stream(page_index, object, color) {
                        Ok(previous) => Ok((
                            current.text,
                            crate::document::TextStyle { color: Some(previous), ..Default::default() },
                        )),
                        // Refused, not quietly handed to PDFium — see the
                        // fast path's own `Err(why)` just below for why.
                        Err(why) => Err(match why {
                            PdfError::Unsupported(_) | PdfError::InvalidArgument(_) => why,
                            _ => PdfError::Unsupported(
                                "this run's colour cannot be changed without rewriting \
                                 the page around it, so it has been left alone",
                            ),
                        }),
                    };
                }
            }
        }

        let only_words_face_or_size =
            crate::document::TextStyle { face: None, size: None, ..style.clone() }
                == crate::document::TextStyle::default();
        if only_words_face_or_size {
            match self.set_run_in_stream(page_index, object, text, style.face.as_deref(), style.size) {
                Ok((previous, was_size)) => {
                    return Ok((
                        previous,
                        crate::document::TextStyle { size: Some(was_size), ..Default::default() },
                    ))
                }
                // **Refused, not quietly handed to PDFium.**
                //
                // Falling through here was the bug: `FPDFText_SetText` needs
                // `FPDFPage_GenerateContent`, which re-emits the *whole* page —
                // so an edit this could not make precisely came out as an edit
                // plus a rewritten paragraph nobody had touched. Reported from
                // use exactly that way, twice.
                //
                // Measured on a real catalogue, this path handles 24 runs in
                // 29. The other five are refused with the reason, which is a
                // worse day than editing them and a far better one than a page
                // that silently changed.
                Err(why) => {
                    // An error that already says something useful is passed
                    // through. Flattening every refusal into one sentence threw
                    // away the only part a person could act on — which
                    // character, and why it is not available.
                    return Err(match why {
                        PdfError::Unsupported(_) | PdfError::InvalidArgument(_) => why,
                        _ => PdfError::Unsupported(
                            "this run cannot be changed without rewriting the page \
                             around it, so it has been left alone",
                        ),
                    });
                }
            }
        }

        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let space = raw.space()?;
        let bindings = pdfium()?.bindings();

        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
        let index = i32::try_from(object).map_err(|_| {
            PdfError::InvalidArgument(format!("object {object} is out of range"))
        })?;
        if index < 0 || index >= count {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has no object {object}",
                page_index + 1
            )));
        }

        let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
        if handle.is_null() {
            return Err(PdfError::Pdfium("that object could not be read".into()));
        }
        if unsafe { bindings.FPDFPageObj_GetType(handle) }
            != pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
        {
            return Err(PdfError::InvalidArgument("that is not text".into()));
        }

        // What the rest of the page says, and a way back to it — see the check
        // after the re-emission below.
        let untouched = {
            let mut out: Vec<String> = self
                .text_runs(page_index)?
                .iter()
                .filter(|r| r.object != object)
                .map(|r| r.text.trim().to_string())
                .collect();
            out.sort();
            out
        };
        let snapshot = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        // What is there now, read through **this** page handle.
        //
        // Not `self.text_runs`, which opens the page again: two live
        // `FPDF_PAGE` handles for one page, with this one about to regenerate
        // its content stream, is how a page ends up with its text drawn twice
        // in two different fonts. PDFium does not arbitrate between them — the
        // second handle simply does not know what the first has done.
        let previous = {
            let text_page = unsafe { bindings.FPDFText_LoadPage(raw.handle) };
            if text_page.is_null() {
                String::new()
            } else {
                let wanted =
                    unsafe { bindings.FPDFTextObj_GetText(handle, text_page, std::ptr::null_mut(), 0) };
                let mut buffer = vec![0u16; (wanted as usize / 2).max(1)];
                let written = unsafe {
                    bindings.FPDFTextObj_GetText(
                        handle,
                        text_page,
                        buffer.as_mut_ptr() as *mut _,
                        wanted,
                    )
                };
                unsafe { bindings.FPDFText_ClosePage(text_page) };
                let len = (written as usize / 2).saturating_sub(1).min(buffer.len());
                String::from_utf16_lossy(&buffer[..len])
            }
        };

        // **What the object looks like, before it is rewritten.**
        //
        // `FPDFText_SetText` writes the object afresh and does not carry the
        // fill colour with it: white text came back black, which on a dark
        // banner means the words vanish. Read here, restored below, and
        // overridden only where the caller asked for something different.
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 255u32);
        let had_colour =
            unsafe { bindings.FPDFPageObj_GetFillColor(handle, &mut r, &mut g, &mut b, &mut a) }
                != 0;

        // Everything the rewrite is about to lose, gathered before it happens
        // and handed back so undo can restore the whole appearance rather than
        // only the words.
        //
        // **`FPDFTextObj_GetFontSize` is the raw `Tf` value, not what the run
        // actually draws at.** A producer that writes `1 Tf` and stretches the
        // text matrix instead still reports "1pt" here — `m.d` is that
        // stretch, the same one `PdfPageTextObject::scaled_font_size` folds
        // in, and `text_runs` (this crate's other reader of a run's size)
        // already reports the corrected value. "The size shown before this
        // edit" and "the size a fresh read of the same run reports" have to
        // be the same number, or undo puts back something nobody asked for.
        let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
        unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
        let vertical_scale = if m.d.abs() > 1e-6 { m.d } else { 1.0 };
        let was = {
            let (x, y) = space.to_top_left(m.e, m.f);
            let mut size = 0.0f32;
            unsafe { bindings.FPDFTextObj_GetFontSize(handle, &mut size) };
            crate::document::TextStyle {
                size: (size > 0.0).then_some(size * vertical_scale),
                color: had_colour.then_some(Color {
                    r: r as u8,
                    g: g as u8,
                    b: b as u8,
                    a: a as u8,
                }),
                at: Some((x, y)),
                face: None,
            }
        };

        // UTF-16LE, null-terminated, as every PDFium string setter wants.
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        utf16.push(0);
        let ok = unsafe { bindings.FPDFText_SetText(handle, utf16.as_ptr()) } != 0;
        if !ok {
            // Almost always a font that cannot draw the new characters. A
            // subsetted font carries only the glyphs the document already used,
            // and a document that never contained a `ß` has no `ß` to draw.
            return Err(PdfError::Unsupported(
                "this run's font cannot write those characters",
            ));
        }

        // Put back what the rewrite did not carry, then apply whatever the
        // caller actually asked to change.
        let colour = style.color.map(|c| {
            (c.r as u32, c.g as u32, c.b as u32, c.a as u32)
        });
        if let Some((r, g, b, a)) = colour.or(had_colour.then_some((r, g, b, a))) {
            unsafe { bindings.FPDFPageObj_SetFillColor(handle, r, g, b, a) };
        }

        if let Some(size) = style.size.filter(|s| *s > 0.0) {
            // `size` is the effective size the caller wants drawn; `Tf` only
            // ever holds the raw value the matrix's own stretch multiplies,
            // so what is actually set has to be divided back down by it —
            // see `vertical_scale`'s own doc, just above.
            unsafe { bindings.FPDFTextObj_SetFontSize(handle, size / vertical_scale) };
        }

        if let Some((x, y)) = style.at {
            // The matrix carries the run's position. Moving it means replacing
            // the translation, not multiplying by another — a second move would
            // otherwise be relative to the first.
            let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
            unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
            let (px, py) = space.to_pdf(x, y);
            m.e = px;
            m.f = py;
            unsafe { bindings.FPDFPageObj_SetMatrix(handle, &m) };
        }

        // Not optional. Without it the change lives in PDFium's object model
        // and never reaches the page's content stream, so it survives until the
        // save and then vanishes.
        if unsafe { bindings.FPDFPage_GenerateContent(raw.handle) } == 0 {
            return Err(PdfError::Pdfium("the page could not be rewritten".into()));
        }
        drop(raw);

        // **And the same guard the move carries**, for the same reason: this is
        // the other path that re-emits a page, and on a real catalogue that
        // came back with its text scrambled. Reported from use as the words
        // sometimes looking changed after an edit — every run *except* the one
        // being edited has to say what it said.
        let rest = |runs: &[crate::document::TextRun]| -> Vec<String> {
            let mut out: Vec<String> = runs
                .iter()
                .filter(|r| r.object != object)
                .map(|r| r.text.trim().to_string())
                .collect();
            out.sort();
            out
        };
        if rest(&self.text_runs(page_index)?) != untouched {
            let restored = Self::open_bytes(snapshot, None)?;
            self.document = restored.document;
            self.page_count = restored.page_count;
            if let Ok(mut cached) = self.vault.lock() {
                *cached = None;
            }
            // Same `Unsupported` suffix trap as the handle-resize message
            // above: worded so "is not implemented yet" lands on the specific
            // thing that refused, not on editing in general (the words
            // themselves are still editable — retyping them does not hit
            // this guard, only a size/colour/position change does).
            return Err(PdfError::Unsupported(
                "changing how these words look rewrites the rest of the page here, so \
                 nothing was changed. Retyping the words themselves does not hit this \
                 guard, only their size, colour or position; doing that on this page",
            ));
        }

        self.touch();
        Ok((previous, was))
    }

    /// Overrides the trait's own one-call-per-edit default with
    /// [`Self::set_runs_in_stream`]'s real batch — but only when every edit
    /// in the group is one `set_text_run_styled` would already send down
    /// its own byte-safe path (see that method's `only_words_face_or_size`
    /// gate); anything else falls back to the default, unchanged, since the
    /// slower PDFium-regenerating path this skips has never been batched
    /// and doing so is out of scope here.
    fn set_text_runs_styled(
        &mut self,
        page_index: usize,
        edits: &[(usize, String, crate::document::TextStyle)],
    ) -> Result<Vec<(String, crate::document::TextStyle)>> {
        // **Atomic per call, not per edit.** `set_text_run_styled` commits
        // each edit to the live document immediately (its own write-and-
        // reopen), so a plain `.map().collect()` here used to stop at the
        // first failure having already written every edit before it —
        // leaving some lines of the same paragraph changed and others not.
        //
        // **The session log of a reported corruption shows the shape of
        // this exactly**: a paragraph apply that succeeded cleanly, then —
        // with no new pick logged in between — a font-encoding error
        // ("'\u{2}' is not in this text's font"), immediately followed by
        // "nothing to undo". That error's own wording only comes from the
        // path a *retype* takes, which an undo reaches too: `revert` calls
        // back in here with the paragraph's *original* text for every
        // line, and if one line's original text carries a character this
        // document's font cannot re-encode (a broken `ToUnicode` entry —
        // see `fix_extracted_text`'s own doc for the same defect read back
        // the other way), that line's own revert fails here. By then
        // `CommandHistory::undo` has already popped this command's undo
        // record *before* calling `revert` — see its own doc for why that
        // makes a command's own revert the last chance to be all-or-
        // nothing — matching "nothing to undo" right after. Reproducing
        // the exact failing edit was not achieved directly, but the
        // fallback's own lack of atomicity is confirmed by inspection
        // regardless: a half-reverted paragraph with its undo record
        // already spent is exactly the "right words, wrong colour"
        // corruption reported, and is possible here independent of
        // whatever specific edit triggers the underlying re-encode
        // failure.
        //
        // So every edit this fallback already committed is itself reverted,
        // in reverse order, before the original error is handed up — the
        // same guarantee `Command::Batch` already gives its own steps, now
        // given to this one's internal fallback too.
        let fallback = |doc: &mut Self| -> Result<Vec<(String, crate::document::TextStyle)>> {
            let mut done: Vec<(usize, String, crate::document::TextStyle)> = Vec::with_capacity(edits.len());
            for (object, text, style) in edits {
                match doc.set_text_run_styled(page_index, *object, text, style) {
                    Ok((previous_text, previous_style)) => done.push((*object, previous_text, previous_style)),
                    Err(e) => {
                        for (object, previous_text, previous_style) in done.into_iter().rev() {
                            let _ = doc.set_text_run_styled(page_index, object, &previous_text, &previous_style);
                        }
                        return Err(e);
                    }
                }
            }
            Ok(done.into_iter().map(|(_, text, style)| (text, style)).collect())
        };

        if edits.len() <= 1 {
            return fallback(self);
        }
        // **Two fast shapes, not one** — the same two `set_text_run_styled`
        // itself already recognises, just checked per edit here instead of
        // once: an ordinary retype (`color`/`at` both absent — face/size may
        // be anything, see `only_words_face_or_size`'s own doc above), or a
        // colour-only change (`face`/`size`/`at` all absent, `color` set) —
        // see `set_runs_in_stream`'s own doc for why the second one matters
        // here specifically.
        let all_fast_path = edits.iter().all(|(_, _, style)| {
            let text_edit = style.color.is_none() && style.at.is_none();
            let color_only =
                style.face.is_none() && style.size.is_none() && style.at.is_none() && style.color.is_some();
            text_edit || color_only
        });
        if !all_fast_path {
            return fallback(self);
        }

        let requests: Vec<(usize, &str, Option<&str>, Option<f32>, Option<crate::document::Color>)> = edits
            .iter()
            .map(|(object, text, style)| {
                (*object, text.as_str(), style.face.as_deref(), style.size, style.color)
            })
            .collect();
        match self.set_runs_in_stream(page_index, &requests, &[], &HashMap::new()) {
            Ok(results) => Ok(results),
            Err(PdfError::Unsupported(msg)) if msg == TOO_MANY_EMBEDS_IN_ONE_BATCH => fallback(self),
            Err(e) => Err(e),
        }
    }

    /// See [`DocumentMut::replace_text_lines`].
    ///
    /// One pass over the page's own content stream: each first piece is written
    /// as [`Self::set_runs_in_stream`] writes an edit, and every piece to remove
    /// is cut out in the same splice, so the page is opened, rewritten and
    /// reopened once however many lines there are — and a refusal anywhere in the
    /// batch happens before anything is written.
    fn replace_text_lines(
        &mut self,
        page_index: usize,
        edits: &[crate::document::TextLineEdit],
    ) -> Result<()> {
        use crate::document::TextLineEdit;

        let mut requests: Vec<(usize, &str, Option<&str>, Option<f32>, Option<crate::document::Color>)> =
            Vec::new();
        let mut removals: Vec<usize> = Vec::new();
        // The width each stretched line is to span, by its first piece.
        let mut stretch: HashMap<usize, f32> = HashMap::new();
        for edit in edits {
            match edit {
                TextLineEdit::Retype { first, text, style, remove, justify_to } => {
                    if let Some(width) = justify_to {
                        if !(width.is_finite() && *width > 0.0) {
                            return Err(PdfError::InvalidArgument(format!(
                                "a line cannot be stretched to a width of {width} points"
                            )));
                        }
                        stretch.insert(*first, *width);
                    }
                    // `set_runs_in_stream` reads a colour as "recolour this and
                    // leave its words", so one asked for here would drop the new
                    // words without a word — and the only other way to write both
                    // is PDFium regenerating the whole page, which is neither
                    // byte-safe nor something a removal can be atomic with.
                    if style.color.is_some() || style.at.is_some() {
                        return Err(PdfError::Unsupported(
                            "retyping a line and changing its colour or position in the same step",
                        ));
                    }
                    requests.push((*first, text.as_str(), style.face.as_deref(), style.size, None));
                    removals.extend(remove.iter().copied());
                }
                TextLineEdit::Remove { objects } => removals.extend(objects.iter().copied()),
            }
        }
        // Every object once, whatever it is asked to do: two things asked of one
        // object cannot both be done, and which was meant is not for this to guess.
        let mut seen = std::collections::HashSet::new();
        for object in requests.iter().map(|r| r.0).chain(removals.iter().copied()) {
            if !seen.insert(object) {
                return Err(PdfError::InvalidArgument(format!(
                    "object {object} is named more than once in one edit of lines"
                )));
            }
        }
        if requests.is_empty() && removals.is_empty() {
            return Ok(());
        }
        self.validate_page_index(page_index)?;

        match self.set_runs_in_stream(page_index, &requests, &removals, &stretch) {
            Ok(_) => Ok(()),
            // Two lines that each need a font written into the file: the batch
            // writes one, so these go one line at a time.
            Err(PdfError::Unsupported(msg)) if msg == TOO_MANY_EMBEDS_IN_ONE_BATCH => {
                self.replace_text_lines_one_at_a_time(page_index, &requests, &removals, &stretch)
            }
            Err(e) => Err(e),
        }
    }


    fn set_page_crop(&mut self, index: usize, crop: Rect) -> Result<()> {
        self.validate_page_index(index)?;
        let page_number = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;
        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();

        let (l, t) = space.to_pdf(crop.left.min(crop.right), crop.top.min(crop.bottom));
        let (r, b) = space.to_pdf(crop.left.max(crop.right), crop.top.max(crop.bottom));
        let (left, right) = (l.min(r), l.max(r));
        let (bottom, top) = (t.min(b), t.max(b));

        // A crop with no area is not a page. Refused rather than clamped to
        // something arbitrary: there is no sensible rectangle to guess at, and a
        // page that renders as nothing looks exactly like a bug in the renderer.
        if (right - left) < 1.0 || (top - bottom) < 1.0 {
            return Err(PdfError::InvalidArgument(
                "a crop must be at least a point across".into(),
            ));
        }

        unsafe { bindings.FPDFPage_SetCropBox(page.handle, left, bottom, right, top) };
        // The boundary lives in the page dictionary, not in its content stream,
        // so there is no content to regenerate — but the page must still be
        // marked changed or the save will not carry it.
        self.touch();
        Ok(())
    }

    // The one real implementation of each is the `Document` impl above —
    // these just give the command stack, which only ever sees a
    // `&mut dyn DocumentMut`, a way to reach it.
    fn move_object_mut(&mut self, page_index: usize, object: usize, by: Point) -> Result<()> {
        <Self as Document>::move_object(self, page_index, object, by)
    }

    fn scale_object_mut(
        &mut self,
        page_index: usize,
        object: usize,
        anchor: Point,
        sx: f32,
        sy: f32,
    ) -> Result<()> {
        <Self as Document>::scale_object(self, page_index, object, anchor, sx, sy)
    }

    fn rotate_object_mut(
        &mut self,
        page_index: usize,
        object: usize,
        pivot: Point,
        degrees: f32,
    ) -> Result<()> {
        <Self as Document>::rotate_object(self, page_index, object, pivot, degrees)
    }

    fn remove_object_mut(&mut self, page_index: usize, object: usize) -> Result<()> {
        <Self as Document>::remove_object(self, page_index, object)
    }

    fn remove_objects_mut(&mut self, page_index: usize, objects: &[usize]) -> Result<()> {
        <Self as Document>::remove_objects(self, page_index, objects)
    }

    fn split_run_into_characters_mut(&mut self, page_index: usize, object: usize) -> Result<()> {
        <Self as Document>::split_run_into_characters(self, page_index, object)
    }

    fn redact(
        &mut self,
        request: &Redaction,
        catalogue: Option<&crate::document::glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        self.redact_inner(request, Intent::Apply, catalogue)
    }

    fn replace_page(&mut self, index: usize, pdf: &[u8]) -> Result<()> {
        self.validate_page_index(index)?;
        let size = self.page_size(index)?;
        self.delete_page(index)?;
        self.insert_page(index, RemovedPage::new(size, pdf.to_vec()))
    }

    fn lock_area(
        &mut self,
        request: &Redaction,
        passcode: &[u8],
        catalogue: Option<&crate::document::glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        // The passcode is answered for first. A wrong one on a document that is
        // already locked must fail here, with the page untouched, rather than
        // after something has been taken off it.
        let (mut vault, dek) = match self.read_vault()? {
            Some(vault) => {
                let dek = vault.unlock(passcode)?;
                (vault, dek)
            }
            None => Vault::create(passcode, KdfParams::default())?,
        };

        // Then the survey, so a page this cannot clear refuses before a sealed
        // copy of it is written into the file.
        self.redact_inner(request, Intent::Survey, catalogue)?;

        // **The way back is written before the thing that needs it.** A failure
        // from here leaves a document carrying a copy of a page that was never
        // changed, which costs bytes and nothing else. The other order has a
        // window where the content is gone and nothing can return it.
        let page = self.snapshot_page(request.page_index)?;
        vault.seal_page(&dek, request.page_index, &page.payload)?;
        // A badge over the area, so a locked passage can be undone by clicking
        // it like anything else. `usize::MAX` for the object because a locked
        // area is not one object — nothing is blanked again when the page comes
        // back, which is right: the redaction is undone by the restore itself.
        vault.hide_shapes(
            request.page_index,
            usize::MAX,
            [
                request.area.left,
                request.area.top,
                request.area.right,
                request.area.bottom,
            ],
            // The shapes themselves, so that unlocking something else on this
            // page — which brings the page back whole — can take these words
            // off it again. The union alone would take more than was picked.
            request
                .shapes()
                .iter()
                .map(|s| [s.left, s.top, s.right, s.bottom])
                .collect(),
        )?;
        self.write_vault(&mut vault, Some(&dek))?;

        // **The marks over the area go too.**
        //
        // Reported from use: "annotations don't get locked". Ink drawn across a
        // passage stayed exactly where it was — drawn on *top* of the black
        // mark — because annotations are not page content. They live in the
        // page's `/Annots` array and a viewer paints them over the page, so
        // cutting the content stream never came near them. Measured on a
        // fixture: after locking an area over a stroke, one annotation left and
        // 17% of the page still inked.
        //
        // Safe to remove because the sealed copy above carries them: unlocking
        // replaces the whole page and the marks come back with it.
        //
        // Done before the content is cut, so it is one save rather than two.
        let mut hidden = 0usize;
        for shape in request.shapes() {
            hidden += self.hide_annotations_in(request.page_index, shape)?;
        }

        // The words come off through `crate::pdf`, which cuts the operators
        // that drew them out of the content stream and copies every other byte
        // through. Redaction's own path would do it through PDFium and so
        // through `FPDFPage_GenerateContent`, which re-emits the whole page —
        // measured on a catalogue page, that reordered a paragraph nobody had
        // touched.
        //
        // Where that cannot be done — a filter this cannot read, a run whose
        // operator cannot be located — it falls back rather than refusing. A
        // lock that works and disturbs the layout is worth more than one that
        // declines, and the original is sealed either way.
        let mut report = match self.hide_area_content(request, catalogue) {
            Ok(report) => report,
            // Shape by shape. PDFium's own path takes one rectangle, and
            // handing it the union of a two-line selection would clear the
            // words either side of what was picked — the very thing the parts
            // exist to avoid.
            Err(e) => return Err(e),
        };
        report.objects += hidden;
        Ok(report)
    }


    fn lock_pages(&mut self, pages: &[usize], passcode: &[u8]) -> Result<usize> {
        if pages.is_empty() {
            return Err(PdfError::InvalidArgument("no pages to lock".into()));
        }

        // The passcode is answered for first, as in `lock_area`: a wrong one on
        // a document that is already locked must fail with every page still
        // where it was.
        let (mut vault, dek) = match self.read_vault()? {
            Some(vault) => {
                let dek = vault.unlock(passcode)?;
                (vault, dek)
            }
            None => Vault::create(passcode, KdfParams::default())?,
        };

        // Sorted and deduplicated so the same page named twice is one lock, and
        // every index is checked before a single page is touched — a range with
        // one bad number must refuse outright rather than blank the pages
        // before it.
        let mut wanted: Vec<usize> = pages.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        for index in &wanted {
            self.validate_page_index(*index)?;
        }

        // Sealed, and the vault written, before anything is blanked.
        let mut newly = 0;
        let mut sizes = Vec::with_capacity(wanted.len());
        for index in &wanted {
            let size = self.page_size(*index)?;
            let page = self.snapshot_page(*index)?;
            if vault.seal_page(&dek, *index, &page.payload)? {
                newly += 1;
                // A badge over the whole page, so a locked page can be undone
                // by clicking it like every other lock. Without one the only
                // way back was the `unlock` verb, which is not discoverable
                // from a page that has just gone blank.
                vault.hide_item(
                    *index,
                    usize::MAX,
                    [0.0, 0.0, size.width_pt, size.height_pt],
                )?;
            }
            sizes.push((*index, size));
        }
        self.write_vault(&mut vault, Some(&dek))?;

        for (index, size) in sizes {
            self.delete_page(index)?;
            self.insert_blank_page(index, size, None, crate::document::blank::Ruling::None)?;
        }
        Ok(newly)
    }

    fn lock_image(&mut self, page_index: usize, object: usize, passcode: &[u8]) -> Result<String> {
        // The passcode first, as everywhere else here: a wrong one on a
        // document that is already locked must fail with the image still on the
        // page.
        let (mut vault, dek) = match self.read_vault()? {
            Some(vault) => {
                let dek = vault.unlock(passcode)?;
                (vault, dek)
            }
            None => Vault::create(passcode, KdfParams::default())?,
        };

        let found = self
            .images_on(page_index)?
            .into_iter()
            .find(|i| i.object == object)
            .ok_or_else(|| {
                PdfError::InvalidArgument(format!(
                    "page {} has no image at object {object}",
                    page_index + 1
                ))
            })?;

        // **The page is the way back**, sealed before anything is touched — the
        // same copy `lock_area` takes, and the first seal for a page wins, so
        // locking a second image keeps the copy that still has both. See
        // `SealedItem` for why the image's own bytes are not kept instead.
        let page = self.snapshot_page(page_index)?;
        vault.seal_page(&dek, page_index, &page.payload)?;

        let rect = [found.rect.left, found.rect.top, found.rect.right, found.rect.bottom];
        let id = vault.hide_item(page_index, object, rect)?;
        self.write_vault(&mut vault, Some(&dek))?;

        // Rewritten now, not at save time: see `apply_locks`.
        //
        // **And the badge comes back out if that fails.** The vault is written
        // before the page is changed, so a failure here leaves a document
        // claiming a lock over a picture that is still perfectly visible —
        // reported from use, with the padlock sitting on top of the image it
        // was supposed to have taken away.
        if let Err(why) = self.apply_locks() {
            vault.forget_item(&id);
            let _ = self.write_vault(&mut vault, Some(&dek));
            return Err(why);
        }
        Ok(id)
    }

    fn unlock_item(&mut self, id: &str, passcode: &[u8]) -> Result<()> {
        let Some(mut vault) = self.read_vault()? else {
            return Err(PdfError::InvalidArgument("this document is not locked".into()));
        };
        let dek = vault.unlock(passcode)?;
        let item = vault
            .item(id)
            .ok_or_else(|| PdfError::InvalidArgument(format!("nothing here is locked as {id}")))?
            .clone();

        // The page comes back whole — exactly as it was before any of this,
        // fonts and all — and then everything still locked on it is hidden
        // again. Restoring the object alone cannot work: the edit that hid it
        // went through `FPDFPage_GenerateContent`, which rewrites the whole
        // page, and only replacing the page undoes that.
        let original = vault.open_page(&dek, item.page_index)?;
        self.replace_page(item.page_index, &original)?;

        vault.forget_item(id);
        self.write_vault(&mut vault, Some(&dek))?;

        // **Every *area* still locked on that page has to be taken off it
        // again**, and `apply_locks` cannot do it: that path knows about
        // images, which it finds from the page, and an area is a shape somebody
        // drew that is nowhere on the page to be found. Reported from use as
        // locked text becoming visible again after an image on the same page
        // was unlocked, with its padlock still sitting over it.
        let areas: Vec<(Rect, Vec<Rect>)> = vault
            .items_on(item.page_index)
            .into_iter()
            .filter(|i| i.object == usize::MAX)
            .map(|i| {
                let rect = |r: &[f32; 4]| Rect {
                    left: r[0],
                    top: r[1],
                    right: r[2],
                    bottom: r[3],
                };
                (rect(&i.rect), i.parts.iter().map(rect).collect())
            })
            .collect();
        for (area, parts) in areas {
            let request = Redaction {
                require_complete: false,
                parts,
                ..Redaction::new(item.page_index, area)
            };
            // A page that cannot be re-cleared is worse than one that never
            // was, so this is refused rather than left half-done. The words are
            // back on the page and the badge has been taken off the item that
            // was unlocked, which is a consistent document either way.
            self.hide_area_content(&request, None)?;
        }

        // And everything else that is still locked on it, which `apply_locks`
        // does for the whole document from the vault, through the same rewrite.
        self.apply_locks()?;
        Ok(())
    }

    fn locked_items_on(&self, page_index: usize) -> Result<Vec<crate::document::LockedItem>> {
        let Some(vault) = self.read_vault()? else { return Ok(Vec::new()) };
        // A locked picture is replaced by a one-pixel mask, so one that is any
        // bigger than that was never taken off the page.
        let pictures = self.images_on(page_index).unwrap_or_default();
        Ok(vault
            .items_on(page_index)
            .into_iter()
            .map(|i| {
                let is_area = i.object == usize::MAX;
                let stale = !is_area
                    && pictures
                        .iter()
                        .find(|p| p.object == i.object)
                        .is_some_and(|p| p.pixel_width > 1 || p.pixel_height > 1);
                crate::document::LockedItem {
                    id: i.id.clone(),
                    rect: crate::document::Rect {
                        left: i.rect[0],
                        top: i.rect[1],
                        right: i.rect[2],
                        bottom: i.rect[3],
                    },
                    // `usize::MAX` is how a locked region is recorded — it
                    // names no object, because it is not one.
                    is_area,
                    stale,
                }
            })
            .collect())
    }

    fn repair_locks(&mut self) -> Result<(usize, usize)> {
        let Some(mut vault) = self.read_vault()? else { return Ok((0, 0)) };
        let stale_before: Vec<String> = (0..self.page_count)
            .flat_map(|page| self.locked_items_on(page).unwrap_or_default())
            .filter(|i| i.stale)
            .map(|i| i.id)
            .collect();
        if stale_before.is_empty() {
            return Ok((0, 0));
        }

        // **Finish the job first.** The picture is identified exactly now, so
        // the blanking that failed once has every chance of going through —
        // and a lock the reader asked for is better completed than forgotten.
        let _ = self.apply_locks();

        let still_stale: Vec<String> = (0..self.page_count)
            .flat_map(|page| self.locked_items_on(page).unwrap_or_default())
            .filter(|i| i.stale)
            .map(|i| i.id)
            .collect();
        let completed = stale_before.len() - still_stale.len();

        // What still cannot be taken off the page is not locked, whatever the
        // badge says. The badge goes; the sealed copy stays, harmlessly.
        if !still_stale.is_empty() {
            vault = self.read_vault()?.unwrap_or(vault);
            for id in &still_stale {
                vault.forget_item(id);
            }
            // No passcode in hand here — this only ever drops a lock nothing
            // can find its way back to. See `write_vault`.
            self.write_vault(&mut vault, None)?;
        }
        Ok((completed, still_stale.len()))
    }

    fn open_lock(&mut self, passcode: &[u8]) -> Result<Vec<(usize, Vec<u8>)>> {
        let Some(mut vault) = self.read_vault()? else {
            return Err(PdfError::InvalidArgument("this document is not locked".into()));
        };
        let dek = vault.unlock(passcode)?;

        // Every tag verified before a single page is handed back. One damaged
        // seal fails the whole call rather than restoring some pages and leaving
        // nobody able to say which are which.
        let pages: Vec<(usize, Vec<u8>)> = vault
            .locked_pages()
            .into_iter()
            .map(|index| vault.open_page(&dek, index).map(|bytes| (index, bytes)))
            .collect::<Result<_>>()?;

        // The caller is putting every one of these pages fully back, so
        // nothing sealed on them is still locked — every badge and every way
        // back goes, the same as `unlock_item` drops the one item it restores.
        // Left alone, a badge (and for a picture, the still-recorded lock
        // itself) survived a whole-document unlock: the passage was back on
        // the page but its padlock was not, and a locked picture whose page
        // seal `apply_locks` could still find stayed hidden regardless.
        for (index, _) in &pages {
            for id in vault.items_on(*index).iter().map(|i| i.id.clone()).collect::<Vec<_>>() {
                vault.forget_item(&id);
            }
            vault.forget_page(*index);
        }
        self.write_vault(&mut vault, Some(&dek))?;

        Ok(pages)
    }

    fn locked_pages(&self) -> Result<Vec<usize>> {
        Ok(self.read_vault()?.map(|v| v.locked_pages()).unwrap_or_default())
    }

    fn preview_redaction(
        &mut self,
        request: &Redaction,
        catalogue: Option<&crate::document::glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        self.redact_inner(request, Intent::Survey, catalogue)
    }

    fn secure_document(
        &mut self,
        user: &[u8],
        owner: Option<&[u8]>,
        permissions: crate::pdf::encrypt::Permissions,
    ) -> Result<()> {
        // **Refused now, not at save time.**
        //
        // A second password cannot be written over a first — the content would
        // be encrypted twice and open for nobody — so the writer refuses it.
        // But it refused at *save*, long after the person had typed a password
        // twice and been told it was set. The answer has to come while they are
        // still asking the question.
        // A password already on the file blocks a second one — but not once it
        // is on its way off. `unsecure` then `secure` is how a password is
        // changed, and checking the raw flag here made that impossible while
        // every layer above it believed otherwise.
        if self.already_has_password() {
            return Err(PdfError::InvalidArgument(
                "this document already has a password — take it off first with \
                 `unsecure`, or use `secure` again to change it."
                    .into(),
            ));
        }
        // Checked now rather than at save time, so a password that cannot be
        // used is refused while the person is still typing it.
        crate::pdf::encrypt::Security::new(user, owner, permissions, Self::randomness)?;
        self.security = Some(Wanted {
            user: zeroize::Zeroizing::new(user.to_vec()),
            owner: owner.map(|o| zeroize::Zeroizing::new(o.to_vec())),
            permissions,
        });
        // PDF's own handler, so any reader can ask for it.
        self.secure_plus = false;
        self.touch();
        Ok(())
    }

    fn secure_document_plus(&mut self, user: &[u8]) -> Result<()> {
        if self.already_has_password() {
            return Err(PdfError::InvalidArgument(
                "this document already has a password — take it off first with \
                 `unsecure`, or use `secure` again to change it."
                    .into(),
            ));
        }
        // Checked now rather than at save time, so a password that cannot be
        // used is refused while the person is still typing it.
        crate::pdf::secure_plus::SecurePlus::new(
            user,
            crate::crypto::kdf::KdfParams::default(),
            Self::randomness,
        )?;
        self.security = Some(Wanted {
            user: zeroize::Zeroizing::new(user.to_vec()),
            owner: None,
            permissions: crate::pdf::encrypt::Permissions::all(),
        });
        self.secure_plus = true;
        self.touch();
        Ok(())
    }

    fn is_secure_plus(&self) -> bool {
        self.secure_plus
    }

    fn unsecure_document(&mut self) -> Result<()> {
        self.secure_plus = false;
        // Two different things wear the same word. A password *waiting* to be
        // written is dropped; a password already *on the file* is marked to
        // come off when it is next saved. Only when there is neither is there
        // nothing to do.
        let had_pending = self.security.take().is_some();
        let has_existing = self.already_secured && !self.remove_password;
        if !had_pending && !has_existing {
            return Err(PdfError::InvalidArgument(
                "this document carries no password".into(),
            ));
        }
        if has_existing {
            self.remove_password = true;
        }
        self.touch();
        Ok(())
    }

    fn is_secured(&self) -> bool {
        self.security.is_some()
    }

    fn had_password_on_open(&self) -> bool {
        self.already_secured
    }

    fn password_matches(&self, typed: &[u8]) -> bool {
        self.opened_with.as_deref().map(|p| p.as_slice()) == Some(typed)
    }

    fn already_has_password(&self) -> bool {
        // A password on its way off is not one in the way: `unsecure` then
        // `secure` is how a password is changed, and refusing the second half
        // would make that impossible.
        self.already_secured && !self.remove_password
    }

    fn sign_document(
        &mut self,
        pkcs12: &[u8],
        password: &str,
        about: &crate::pdf::sign::Reason,
    ) -> Result<String> {
        let identity = crate::pdf::sign::Identity::from_pkcs12(pkcs12, password)?;
        let who = identity.subject().unwrap_or_default();

        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let signed = crate::pdf::sign::sign(&file, &identity, about)?;

        // Kept before the reopen consumes it: this is the only moment the
        // exact signed bytes exist, and validation needs exactly these.
        let exact = signed.clone();
        let reopened = Self::open_bytes(signed, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        self.written = Some(exact);
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        // **The signed bytes are the file, and they are not on disk yet.** A
        // save writes them verbatim — see `exact_pending` — so the document is
        // dirty, and closing it now would lose the signature.
        self.dirty = true;
        self.exact_pending = true;
        self.exact_content = true;
        Ok(who)
    }

    fn signature_count(&self) -> usize {
        PdfiumDocument::signature_count(self).max(0) as usize
    }

    fn validate_signatures(&mut self) -> Result<Vec<crate::pdf::validate::Signature>> {
        // **Not** `save_to_bytes`. A signature covers a stretch of one exact
        // file, and asking PDFium to write this document out produces a
        // different one — which reads as a signature covering a fraction of
        // the file, rather than as the correctly signed document it is.
        //
        // `self.written` is trusted only while `exact_content` says nothing
        // inside PDFium has changed since it was captured. An annotation or
        // an edit leaves it holding a file that is no longer this document —
        // reporting against it describes bytes nobody can see or save
        // anymore. The file on disk, when there is one, is whatever was last
        // genuinely saved: stale relative to an unsaved edit too, but at
        // least a real file that still exists, rather than a snapshot that
        // now matches neither the disk nor the document in memory. Found by
        // audit.
        let bytes = if let Some(exact) = self.written.as_ref().filter(|_| self.exact_content) {
            exact.clone()
        } else if let DocumentSource::Path(path) = &self.source {
            std::fs::read(path)?
        } else if let Some(exact) = &self.written {
            // No path to fall back to — a document opened from bytes rather
            // than a file. Stale is still better than nothing to check.
            exact.clone()
        } else {
            // No file to read and no bytes kept — say that rather than check
            // something else and present the answer as being about this.
            return Err(PdfError::Unsupported(
                "checking signatures on a document that has never been written",
            ));
        };
        let file = crate::pdf::File::parse(&bytes)?;
        crate::pdf::validate::check(&file, &bytes)
    }

    fn set_typing_fonts(&mut self, fonts: Vec<Vec<u8>>) {
        self.typing_fonts = fonts;
    }

    fn add_typing_font(&mut self, font: Vec<u8>) {
        // A caller retrying several runs against the same on-page font asks
        // for it again every time — reported from use as the app freezing
        // solid on a large document. With no check here, `typing_fonts` grew
        // one full duplicate copy of the font's bytes per attempt rather than
        // per distinct font, and every later `embed_typing_font` search below
        // re-parses every entry in it — so a selection needing many retypes
        // got slower with each one, compounding into what looked like a
        // hang. The same bytes arriving again is the common case (the exact
        // data `registered_face_for_run` just re-read for the same run), so
        // a plain equality check catches it without parsing anything.
        if self.typing_fonts.iter().any(|existing| existing == &font) {
            return;
        }
        self.typing_fonts.push(font);
    }

    fn stamp_line(&mut self, page_index: usize, from: Point, to: Point) -> Result<()> {
        self.validate_page_index(page_index)?;
        if (from.x - to.x).hypot(from.y - to.y) < 0.5 {
            return Err(PdfError::InvalidArgument("that line has no length".into()));
        }
        let height = self.page_size(page_index)?.height_pt;
        let painted = fill_line(from, to, height);
        self.append_to_page(page_index, &painted)
    }

    fn stamp_box(&mut self, page_index: usize, area: Rect) -> Result<()> {
        self.validate_page_index(page_index)?;
        if area.right <= area.left || area.bottom <= area.top {
            return Err(PdfError::InvalidArgument("that area has no size".into()));
        }
        let height = self.page_size(page_index)?.height_pt;
        let painted = fill_box(area, height);
        self.append_to_page(page_index, &painted)
    }

    fn mark_as_signature(&mut self, page_index: usize, index: usize, name: &str) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let annot_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("annotation index {index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, annot_index) };
        if annot.is_null() {
            return Err(PdfError::Pdfium(format!(
                "page {page_index} has no annotation at index {index}"
            )));
        }
        // Refused rather than written onto whatever is at that index. Marking a
        // highlight as a signature would have it flattened into the page by a
        // tool the user thought was only touching their own name.
        let subtype = unsafe { bindings.FPDFAnnot_GetSubtype(annot) };
        if subtype != ANNOT_INK && subtype != ANNOT_STAMP {
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };
            return Err(PdfError::InvalidArgument(
                "only a drawn or uploaded signature can be marked as one".into(),
            ));
        }

        let mut value: Vec<u16> = name.encode_utf16().collect();
        value.push(0);
        let set = unsafe { bindings.FPDFAnnot_SetStringValue(annot, SIGNATURE_KEY, value.as_ptr()) };
        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        if set == 0 {
            return Err(PdfError::Pdfium("the signature could not be marked".into()));
        }

        // A string on an existing annotation, not page content.
        self.touch_annotation();
        Ok(())
    }

    fn remember_image_alpha(&mut self, page_index: usize, index: usize, rgba: Vec<u8>) -> Result<()> {
        // Nothing to keep for a picture that is already fully opaque — every
        // upload made before signature extraction produced real alpha
        // reaches here this way, and there is nothing a soft mask would add.
        if rgba.chunks_exact(4).all(|p| p[3] == 255) {
            return Ok(());
        }
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let annot_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("annotation index {index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, annot_index) };
        if annot.is_null() {
            return Err(PdfError::Pdfium(format!(
                "page {page_index} has no annotation at index {index}"
            )));
        }

        // The id, not the pixels, goes on the annotation — the pixels stay
        // in this process. See `image_alpha`'s own doc for why.
        let id = self.next_alpha_id;
        self.next_alpha_id += 1;
        let mut value: Vec<u16> = id.to_string().encode_utf16().collect();
        value.push(0);
        let set = unsafe { bindings.FPDFAnnot_SetStringValue(annot, ALPHA_ID_KEY, value.as_ptr()) };
        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        if set == 0 {
            return Err(PdfError::Pdfium("the signature's pixels could not be kept".into()));
        }

        self.image_alpha.insert(id, rgba);
        // A string on an existing annotation, not page content.
        self.touch_annotation();
        Ok(())
    }

    fn set_image_signature_rect(&mut self, page_index: usize, index: usize, rect: Rect) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let annot_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("annotation index {index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let space = page.space()?;
        let bindings = pdfium()?.bindings();
        let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, annot_index) };
        if annot.is_null() {
            return Err(PdfError::Pdfium(format!(
                "page {page_index} has no annotation at index {index}"
            )));
        }

        // Moving or resizing must not silently un-rotate a picture that was
        // already turned — the angle lives only in `ROTATION_KEY`, so it has
        // to be read back here and carried into the new matrix, not assumed
        // to be zero.
        let rotation = read_rotation(annot);
        let pdf_rect = to_pdf_rect(&space, &rect);
        let set_rect = unsafe { bindings.FPDFAnnot_SetRect(annot, &pdf_rect) };
        // The annotation's own `/Rect` is what a click hits and what any
        // other reader shows the picture at; the image object's matrix,
        // fetched separately, is what actually draws the pixels — both have
        // to move together or the two disagree about where the picture is.
        let object = unsafe { bindings.FPDFAnnot_GetObject(annot, 0) };
        let set_matrix = if object.is_null() {
            0
        } else {
            let matrix = image_placement_matrix(&pdf_rect, rotation);
            unsafe { bindings.FPDFPageObj_SetMatrix(object, &matrix) }
        };
        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        if set_rect == 0 || set_matrix == 0 {
            return Err(PdfError::Pdfium("the signature could not be moved".into()));
        }

        self.touch_annotation();
        Ok(())
    }

    fn rotate_image_signature(&mut self, page_index: usize, index: usize, degrees: f32) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let annot_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("annotation index {index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, annot_index) };
        if annot.is_null() {
            return Err(PdfError::Pdfium(format!(
                "page {page_index} has no annotation at index {index}"
            )));
        }

        // The rect itself never changes here — only the angle the picture
        // is drawn at within it, about its own centre. Read back rather
        // than assumed, for the same reason `set_image_signature_rect`
        // reads the rotation back: rotating must not silently undo a move
        // or a resize that happened first.
        let mut pdf_rect = FS_RECTF { left: 0.0, bottom: 0.0, right: 0.0, top: 0.0 };
        let got_rect = unsafe { bindings.FPDFAnnot_GetRect(annot, &mut pdf_rect) };
        if got_rect == 0 {
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };
            return Err(PdfError::Pdfium("the signature's own position could not be read".into()));
        }

        let mut value: Vec<u16> = degrees.to_string().encode_utf16().collect();
        value.push(0);
        let set_key = unsafe { bindings.FPDFAnnot_SetStringValue(annot, ROTATION_KEY, value.as_ptr()) };

        let object = unsafe { bindings.FPDFAnnot_GetObject(annot, 0) };
        let set_matrix = if object.is_null() {
            0
        } else {
            let matrix = image_placement_matrix(&pdf_rect, degrees);
            unsafe { bindings.FPDFPageObj_SetMatrix(object, &matrix) }
        };
        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        if set_key == 0 || set_matrix == 0 {
            return Err(PdfError::Pdfium("the signature could not be rotated".into()));
        }

        self.touch_annotation();
        Ok(())
    }

    fn apply_signatures(&mut self, page_index: usize) -> Result<usize> {
        let ink_marks = self.signature_marks(page_index)?;
        let image_marks = self.image_signature_marks(page_index)?;
        let total = ink_marks.len() + image_marks.len();
        if total == 0 {
            return Ok(0);
        }
        let height = self.page_size(page_index)?.height_pt;

        // One append for the whole page rather than one per signature: each
        // append writes the document out and parses it again, which on a large
        // file is the expensive part and has nothing to do with how many
        // signatures are on the page. A picture needs more than an append —
        // it needs its own XObject in the file and a name for it in the
        // page's `/Resources` — so this does not go through `append_to_page`,
        // which has nowhere to put that. Built here instead, in the same
        // `edit_base`/`write_edit`/`adopt_edit` shape, so a signed document's
        // bytes still move only by appending. See `fill_image_annotation`'s
        // doc for why *placing* a picture does not need this and *applying*
        // one does.
        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let mut base = self.edit_base()?;

        // **An annotation this is about to consume is not the only thing
        // that might be live and unsaved.** `base.bytes`, when exact, is
        // frozen at the last sign or real save — it never gained an
        // `/Annots` entry for anything placed since, on any page, because
        // nothing here ever serialises one into `written`. Reopening onto it
        // (in `adopt_edit`) therefore shows only what was there *then* — the
        // marks about to be applied included, which is exactly why the
        // removal loop below must not run against that reopened document by
        // index: those indices describe the *live* document, not it. If
        // anything besides these marks is also pending — another page's
        // drawing not yet saved, say — reopening onto the exact base would
        // silently drop it too. So this is checked, page by page, before the
        // exact base is trusted; a mismatch anywhere falls back to a real
        // rewrite, correct and merely not appended, rather than risk losing
        // somebody's unsaved mark.
        if base.exact {
            let file = crate::pdf::File::parse(&base.bytes)?;
            let counted = annots_counts(&file)?;
            let mut safe = counted.len() == self.page_count;
            for (p, from_base) in counted.into_iter().enumerate() {
                if !safe {
                    break;
                }
                let expected = from_base + if p == page_index { total } else { 0 };
                safe = self.annotation_count(p)? == expected;
            }
            if !safe {
                base = EditBase { bytes: self.readable_bytes()?, exact: false };
            }
        }

        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, page_index)?;

        let mut painted = Vec::new();
        for mark in &ink_marks {
            painted.extend_from_slice(&ink_operators(&mark.strokes, mark.color, mark.width, height));
        }

        let mut extra: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut next_number = file.next_object_number()?;

        // A picture's own object, and its own name in the page's Resources —
        // on the page itself, not whatever it inherits, for the same reason
        // `embed_font` does this: the ancestor may be shared with other pages,
        // and this name must mean something only here.
        let mut page_dict: Option<crate::pdf::Dict> = None;
        if !image_marks.is_empty() {
            let mut resources = self
                .inherited(&file, &page, b"Resources")?
                .and_then(|r| r.as_dict().cloned())
                .unwrap_or(crate::pdf::Dict(Vec::new()));
            let mut xobjects = resources
                .get(b"XObject")
                .and_then(|x| file.resolve(x).ok())
                .and_then(|x| x.as_dict().cloned())
                .unwrap_or(crate::pdf::Dict(Vec::new()));

            for mark in &image_marks {
                let mut resource = b"PagifyImage1".to_vec();
                for suffix in 1..=64u32 {
                    let candidate = format!("PagifyImage{suffix}").into_bytes();
                    if xobjects.get(&candidate).is_none() {
                        resource = candidate;
                        break;
                    }
                }
                // A soft mask only when there is real transparency to carry
                // — the common case (an ink signature never reaches here at
                // all; a plain, pre-extraction upload is uniformly opaque)
                // gets exactly the single-object picture it always did.
                let smask = if mark.rgba.chunks_exact(4).any(|p| p[3] != 255) {
                    let number = next_number;
                    next_number += 1;
                    extra.push((number, alpha_smask_xobject(&mark.rgba, mark.width, mark.height)?));
                    Some(number)
                } else {
                    None
                };
                let number = next_number;
                next_number += 1;
                extra.push((number, image_xobject(&mark.rgba, mark.width, mark.height, smask)?));
                xobjects.set(&resource, crate::pdf::Object::Reference(number, 0));
                painted.extend_from_slice(&place_image_operators(&resource, mark.rect, height, mark.rotation));
            }

            resources.set(b"XObject", crate::pdf::Object::Dict(xobjects));
            let mut dict = page
                .as_dict()
                .cloned()
                .ok_or(PdfError::Unsupported("that page cannot be read"))?;
            dict.set(b"Resources", crate::pdf::Object::Dict(resources));
            page_dict = Some(dict);
        }

        let mut replacements = Vec::new();
        match self.page_content(&file, &page) {
            Ok((mut stream, streams)) => {
                stream.extend_from_slice(&painted);
                for (index, (number, dict)) in streams.iter().enumerate() {
                    let data = if index == 0 { stream.clone() } else { Vec::new() };
                    let packed = crate::pdf::content::encode(&data)?;
                    let mut dict = dict.clone();
                    dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
                    dict.remove(b"DecodeParms");
                    replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
                }
                if let Some(dict) = page_dict {
                    let page_number = self.page_object_number(&file, page_index)?;
                    let mut body = Vec::new();
                    crate::pdf::write_object(&mut body, &crate::pdf::Object::Dict(dict));
                    replacements.push((page_number, body));
                }
            }
            Err(_) => {
                // No content stream: make one, exactly as `append_to_page`
                // does — and, if a picture already needs the page dict
                // rewritten for its Resources, set `/Contents` on that same
                // dict rather than writing the page twice.
                let number = next_number;
                next_number += 1;
                let packed = crate::pdf::content::encode(&painted)?;
                let mut dict = crate::pdf::Dict(Vec::new());
                dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
                extra.push((number, crate::pdf::write_stream(&dict, &packed)));

                let page_number = self.page_object_number(&file, page_index)?;
                let mut dict = match page_dict {
                    Some(dict) => dict,
                    None => {
                        let crate::pdf::Object::Dict(dict) = file.object(page_number)? else {
                            return Err(PdfError::InvalidArgument("that page cannot be written".into()));
                        };
                        dict
                    }
                };
                dict.set(b"Contents", crate::pdf::Object::Reference(number, 0));
                let mut body = Vec::new();
                crate::pdf::write_object(&mut body, &crate::pdf::Object::Dict(dict));
                replacements.push((page_number, body));
            }
        }
        let _ = next_number;

        let base_was_exact = base.exact;
        let rewritten = Self::write_edit(&base, &file, &replacements, &extra)?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)?;

        if base_was_exact {
            // The reopen above already shows the document without these
            // marks: an exact base never had them, by construction — see the
            // check that put us on this path. There is nothing left to
            // remove, and calling `remove_annotation` against indices that
            // described the *pre-reopen* live document would either remove
            // the wrong thing or find nothing at that index at all.
        } else {
            // From the back: removing by index renumbers everything after it.
            // Ink and image marks share one index space — PDFium's own count
            // of annotations on the page — so their indices are merged first.
            let mut indices: Vec<usize> = ink_marks
                .iter()
                .map(|m| m.index)
                .chain(image_marks.iter().map(|m| m.index))
                .collect();
            indices.sort_unstable();
            for index in indices.into_iter().rev() {
                self.remove_annotation(page_index, index)?;
            }
        }
        // These pixels are page content now — the copy kept outside
        // PDFium's own annotation has done its one job and is not needed
        // again. Not required for correctness (an id is never reused), only
        // so this does not grow for the life of the document.
        for mark in &image_marks {
            if let Some(id) = mark.alpha_id {
                self.image_alpha.remove(&id);
            }
        }
        Ok(total)
    }

    fn stamp_mark(
        &mut self,
        page_index: usize,
        mark: crate::document::FillMark,
        at: Point,
        size: f32,
    ) -> Result<()> {
        self.validate_page_index(page_index)?;
        if size <= 0.0 {
            return Err(PdfError::InvalidArgument("that mark has no size".into()));
        }
        let height = self.page_size(page_index)?.height_pt;
        let painted = fill_mark(mark, at, size, height);
        self.append_to_page(page_index, &painted)
    }

    fn set_sensitivity(&mut self, level: crate::document::sensitivity::Sensitivity) -> Result<()> {
        use crate::document::sensitivity::STAMP_ID;
        use crate::document::{Annotation, Glyph};

        // Whatever was there comes off first, so re-marking replaces rather
        // than stacks — a page reading "INTERNAL CONFIDENTIAL" says neither.
        self.clear_sensitivity()?;

        const SIZE: f32 = 10.0;
        for page_index in 0..self.page_count {
            let size = self.page_size(page_index)?;
            let stamp = level.stamp();

            // Top right, inside the margin. Chosen because a top-left stamp
            // lands on the first line of most documents and a centred one lands
            // on a heading; the right-hand margin is the emptiest part of a
            // page that people still look at.
            //
            // The width is estimated at 0.62 em a character, which Helvetica's
            // capitals sit near. Being a point or two out moves the stamp, and
            // moving a stamp is not a failure — running off the page would be,
            // so it is clamped.
            let width = SIZE * 0.62 * stamp.chars().count() as f32;
            let x = (size.width_pt - width - 24.0).max(24.0);
            let y = 24.0;

            self.add_annotation(
                page_index,
                &Annotation::Text {
                    text: stamp.to_string(),
                    font: "Helvetica".into(),
                    font_asset: None,
                    size: SIZE,
                    color: level.colour(),
                    glyphs: vec![Glyph { ch: stamp.to_string(), id: 0, x, y, radians: 0.0 }],
                    // One id for every stamp in the document, which is how the
                    // whole marking comes off in one go.
                    id: STAMP_ID,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                },
            )?;
        }

        self.touch();
        Ok(())
    }

    fn clear_sensitivity(&mut self) -> Result<()> {
        use crate::document::sensitivity::STAMP_ID;

        for page_index in 0..self.page_count {
            // Absent on most pages most of the time; not an error.
            let _ = self.remove_text(page_index, STAMP_ID);
        }
        self.touch();
        Ok(())
    }

    fn sensitivity(&mut self) -> Option<crate::document::sensitivity::Sensitivity> {
        use crate::document::sensitivity::Sensitivity;

        // **The stamp is the record.** There is no second copy in the document
        // information, deliberately: `hiddendata clean` removes that, and a
        // marking that disappeared when somebody sanitised a file would be
        // worse than none. What is on the page is what the document says.
        //
        // Read as a whole run rather than as a substring, so a document
        // *discussing* confidentiality is not mistaken for one marked with it.
        // A page whose own text has a run reading exactly "SECRET" would be —
        // the cost of not keeping a second copy somewhere it could rot.
        let runs = self.text_runs(0).ok()?;
        runs.iter().find_map(|run| {
            Sensitivity::all()
                .into_iter()
                .find(|level| run.text.trim() == level.stamp())
        })
    }

    fn whiteout(&mut self, page_index: usize, area: Rect, colour: Color) -> Result<()> {
        self.validate_page_index(page_index)?;
        if area.right <= area.left || area.bottom <= area.top {
            return Err(PdfError::InvalidArgument("that area has no size".into()));
        }
        let height = self.page_size(page_index)?.height_pt;
        let painted = mark(area, height, colour);
        self.append_to_page(page_index, &painted)
    }

    fn hidden_data(&mut self) -> Result<crate::pdf::hidden::Hidden> {
        // The bytes PDFium would write, not the bytes on disk: the survey has
        // to describe the document as it stands, including anything done to it
        // since it was opened.
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let mut found = crate::pdf::hidden::survey(&file, &bytes)?;
        self.count_what_only_the_file_shows(&mut found);
        Ok(found)
    }

    fn remove_hidden_data(&mut self) -> Result<crate::pdf::hidden::Sanitised> {
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let (cleaned, mut found) = crate::pdf::hidden::strip(&file, &bytes)?;
        // The full-copy save this forces is what takes the earlier revisions
        // and the orphaned objects out of the file — so they are counted as
        // removed, from the file they are in.
        self.count_what_only_the_file_shows(&mut found.before);

        // Reopened from the sanitised bytes, so what the person is looking at
        // is the document they just cleaned.
        let reopened = Self::open_bytes(cleaned, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        self.touch();
        // The file now holds one revision, and appending to it would start the
        // problem over: an earlier version of every page, in front of the new
        // one.
        self.redacted = true;
        Ok(found)
    }

    fn must_save_full_copy(&self) -> bool {
        // A password cannot be appended: an incremental save leaves the whole
        // original revision in the file, in plain sight, with the encrypted one
        // after it. Nor can one be taken off by appending: the revision added
        // to an encrypted file is encrypted like the rest, so the password a
        // person was told was off stayed on. Found by audit.
        self.redacted || self.security.is_some() || self.remove_password
    }

    fn transform_page(&mut self, index: usize, matrix: [f32; 6]) -> Result<()> {
        self.validate_page_index(index)?;
        let page_number = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;
        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();

        let m = FS_MATRIX {
            a: matrix[0],
            b: matrix[1],
            c: matrix[2],
            d: matrix[3],
            e: matrix[4],
            f: matrix[5],
        };
        // No clip: the whole page moves, and a clip rectangle here would cut the
        // content at the *old* boundary while it is on its way to the new one.
        let ok = unsafe {
            bindings.FPDFPage_TransFormWithClip(page.handle, &m, std::ptr::null())
        } != 0;
        if !ok {
            // PDFium refuses a page with nothing on it, and it is right to —
            // there is no content stream to rewrite. That is not a failure:
            // moving nothing succeeds trivially, and treating it as an error
            // means a blank page cannot be resized along with the rest of the
            // document it belongs to.
            let objects = unsafe { bindings.FPDFPage_CountObjects(page.handle) };
            if objects > 0 {
                return Err(PdfError::Pdfium("this page could not be transformed".into()));
            }
        }
        self.touch();
        Ok(())
    }

    fn set_page_media(&mut self, index: usize, width_pt: f32, height_pt: f32) -> Result<()> {
        self.validate_page_index(index)?;
        if width_pt < 1.0 || height_pt < 1.0 {
            return Err(PdfError::InvalidArgument(
                "a page must be at least a point across".into(),
            ));
        }
        let page_number = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;
        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();

        unsafe {
            bindings.FPDFPage_SetMediaBox(page.handle, 0.0, 0.0, width_pt, height_pt);
            // The crop follows the sheet. Left behind, it would keep showing a
            // window the size of the old page onto the new one.
            bindings.FPDFPage_SetCropBox(page.handle, 0.0, 0.0, width_pt, height_pt);
        }
        self.touch();
        Ok(())
    }

    fn page_rotation(&self, index: usize) -> Result<u8> {
        self.validate_page_index(index)?;
        let page_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;

        let page = self
            .document
            .pages()
            .get(page_index)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        Ok(
            match page
                .rotation()
                .map_err(|e| PdfError::Pdfium(e.to_string()))?
            {
                PdfPageRenderRotation::Degrees90 => 1,
                PdfPageRenderRotation::Degrees180 => 2,
                PdfPageRenderRotation::Degrees270 => 3,
                _ => 0,
            },
        )
    }

    fn add_annotation(&mut self, page_index: usize, annotation: &Annotation) -> Result<usize> {
        self.validate_page_index(page_index)?;

        // **Nothing new goes over a lock.**
        //
        // Locking seals what is on the page at that moment and takes the marks
        // over the area with it. A mark drawn afterwards is not part of that
        // seal, so it would sit on top of the black box, be lost on unlocking,
        // and — worst of the three — look to anyone reading the page like a
        // note about content they cannot see.
        //
        // Refused rather than silently dropped, so the person is told why and
        // can unlock first. Restoring a page does not come through here: it
        // replaces the whole page, marks included.
        if let Some(bounds) = annotation.bounds().filter(|_| !annotation.draws_nothing()) {
            let locked = self.locked_items_on(page_index)?;
            if locked.iter().any(|item| {
                bounds.left < item.rect.right
                    && bounds.right > item.rect.left
                    && bounds.top < item.rect.bottom
                    && bounds.bottom > item.rect.top
            }) {
                return Err(PdfError::InvalidArgument(
                    "that part of the page is locked — unlock it before marking it".into(),
                ));
            }
        }

        let index = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), index)?;

        // Text is not an annotation. It goes into the page's own content so that
        // a reader can select and search it, which means there is no annotation
        // index to hand back and nothing for `remove_annotation` to take away
        // afterwards — see `write_text`. The count is returned unchanged, which is
        // the honest answer: this call added no annotation.
        if let Annotation::Text { .. } = annotation {
            self.write_text(&page, annotation)?;
            self.touch();
            let count =
                unsafe { pdfium()?.bindings().FPDFPage_GetAnnotCount(page.handle) }.max(0);
            return Ok(count as usize);
        }

        self.write_annotation(&page, annotation)?;

        // Read the count back rather than assume. PDFium appends, so the new mark
        // is the last one — but taking the count makes that a measured fact rather
        // than an assumption the undo record silently depends on.
        let count = unsafe { pdfium()?.bindings().FPDFPage_GetAnnotCount(page.handle) };
        if count <= 0 {
            return Err(PdfError::Pdfium(
                "annotation was created but the page reports none".into(),
            ));
        }

        // An `/Annots` entry, not page content — see `exact_content`.
        self.touch_annotation();
        Ok((count - 1) as usize)
    }

    fn remove_annotation(&mut self, page_index: usize, index: usize) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let annot_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("annotation index {index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let removed = unsafe {
            pdfium()?
                .bindings()
                .FPDFPage_RemoveAnnot(page.handle, annot_index)
        };
        if removed == 0 {
            return Err(PdfError::Pdfium(format!(
                "page {page_index} has no annotation at index {index}"
            )));
        }

        // See `exact_content`: removing an annotation is not a content edit.
        self.touch_annotation();
        Ok(())
    }

    fn add_bookmark(&mut self, title: &str, page_index: usize) -> Result<crate::document::BookmarkAdded> {
        use crate::document::BookmarkAdded;
        use crate::pdf::{Dict, Object};

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;

        let page_number = self.page_object_number(&file, page_index)?;

        let Some(Object::Reference(root_number, _)) = file.trailer().get(b"Root") else {
            return Err(PdfError::InvalidArgument("the file has no catalogue".into()));
        };
        let root_number = *root_number;
        let Ok(Object::Dict(mut root)) = file.object(root_number) else {
            return Err(PdfError::InvalidArgument("the catalogue cannot be read".into()));
        };

        // The outline root, made fresh if this document has never had one —
        // see `BookmarkAdded::outlines_is_new`'s own doc for why undo needs
        // to know which of the two happened.
        let outlines_is_new = root.get(b"Outlines").is_none();
        let (outlines_number, mut outlines, previous_last) = if outlines_is_new {
            (file.next_object_number()?, Dict(Vec::new()), None)
        } else {
            let Some(Object::Reference(number, _)) = root.get(b"Outlines") else {
                return Err(PdfError::InvalidArgument(
                    "the catalogue's outline entry is not a reference".into(),
                ));
            };
            let number = *number;
            let Ok(Object::Dict(existing)) = file.object(number) else {
                return Err(PdfError::InvalidArgument("the outline root cannot be read".into()));
            };
            let previous_last = match existing.get(b"Last") {
                Some(Object::Reference(n, _)) => Some(*n),
                _ => None,
            };
            (number, existing, previous_last)
        };
        // One past the root when the root is also new (no other allocation
        // has happened yet to collide with), or a fresh number of its own
        // when the root already existed — never the same call twice, which
        // would hand back the same number both times.
        let item_number =
            if outlines_is_new { outlines_number + 1 } else { file.next_object_number()? };

        let mut item = Dict(Vec::new());
        item.set(b"Title", Object::HexString(utf16be_hex(title)));
        item.set(b"Parent", Object::Reference(outlines_number, 0));
        // `/Fit` rather than a remembered scroll position: naming the page
        // is the part a bookmark is for, and a stored zoom/offset would be
        // one more thing to get wrong reading it back for no benefit anyone
        // asked for.
        item.set(
            b"Dest",
            Object::Array(vec![Object::Reference(page_number, 0), Object::Name(b"Fit".to_vec())]),
        );
        if let Some(prev) = previous_last {
            item.set(b"Prev", Object::Reference(prev, 0));
        }
        let mut item_body = Vec::new();
        crate::pdf::write_object(&mut item_body, &Object::Dict(item));

        let mut replacements = Vec::new();
        let mut extra = vec![(item_number, item_body)];

        if let Some(prev) = previous_last {
            let Ok(Object::Dict(mut prev_dict)) = file.object(prev) else {
                return Err(PdfError::InvalidArgument("the previous bookmark cannot be read".into()));
            };
            prev_dict.set(b"Next", Object::Reference(item_number, 0));
            let mut prev_body = Vec::new();
            crate::pdf::write_object(&mut prev_body, &Object::Dict(prev_dict));
            replacements.push((prev, prev_body));
        } else {
            outlines.set(b"First", Object::Reference(item_number, 0));
        }
        outlines.set(b"Last", Object::Reference(item_number, 0));
        outlines.set(b"Type", Object::Name(b"Outlines".to_vec()));
        let previous_count = outlines.get(b"Count").and_then(Object::as_i64).unwrap_or(0);
        outlines.set(b"Count", Object::Number((previous_count + 1).to_string().into_bytes()));
        let mut outlines_body = Vec::new();
        crate::pdf::write_object(&mut outlines_body, &Object::Dict(outlines));

        if outlines_is_new {
            extra.push((outlines_number, outlines_body));
            root.set(b"Outlines", Object::Reference(outlines_number, 0));
            let mut root_body = Vec::new();
            crate::pdf::write_object(&mut root_body, &Object::Dict(root));
            replacements.push((root_number, root_body));
        } else {
            replacements.push((outlines_number, outlines_body));
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &extra)?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)?;

        Ok(BookmarkAdded { outlines_object: outlines_number, outlines_is_new, item_object: item_number, previous_last })
    }

    fn remove_bookmark(&mut self, added: crate::document::BookmarkAdded) -> Result<()> {
        use crate::pdf::Object;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;

        let mut replacements = Vec::new();

        if let Some(prev) = added.previous_last {
            let Ok(Object::Dict(mut prev_dict)) = file.object(prev) else {
                return Err(PdfError::InvalidArgument("the previous bookmark cannot be read".into()));
            };
            prev_dict.remove(b"Next");
            let mut prev_body = Vec::new();
            crate::pdf::write_object(&mut prev_body, &Object::Dict(prev_dict));
            replacements.push((prev, prev_body));
        }

        if added.outlines_is_new {
            // This bookmark is what created the outline root — undo drops
            // `/Outlines` from the catalogue again rather than leaving an
            // empty one behind.
            let Some(Object::Reference(root_number, _)) = file.trailer().get(b"Root") else {
                return Err(PdfError::InvalidArgument("the file has no catalogue".into()));
            };
            let root_number = *root_number;
            let Ok(Object::Dict(mut root)) = file.object(root_number) else {
                return Err(PdfError::InvalidArgument("the catalogue cannot be read".into()));
            };
            root.remove(b"Outlines");
            let mut root_body = Vec::new();
            crate::pdf::write_object(&mut root_body, &Object::Dict(root));
            replacements.push((root_number, root_body));
        } else {
            let Ok(Object::Dict(mut outlines)) = file.object(added.outlines_object) else {
                return Err(PdfError::InvalidArgument("the outline root cannot be read".into()));
            };
            match added.previous_last {
                Some(prev) => outlines.set(b"Last", Object::Reference(prev, 0)),
                None => {
                    outlines.remove(b"First");
                    outlines.remove(b"Last");
                }
            }
            let previous_count = outlines.get(b"Count").and_then(Object::as_i64).unwrap_or(1);
            outlines
                .set(b"Count", Object::Number((previous_count - 1).max(0).to_string().into_bytes()));
            let mut outlines_body = Vec::new();
            crate::pdf::write_object(&mut outlines_body, &Object::Dict(outlines));
            replacements.push((added.outlines_object, outlines_body));
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    fn take_annotation(&mut self, page_index: usize, index: usize) -> Result<Annotation> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let annot_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("annotation index {index} is out of range"))
        })?;

        // Read it *before* removing it. Afterwards there is nothing left to read,
        // and the undo record would have nothing to put back.
        let taken = {
            let page = RawPage::open(self.document.handle(), page_number)?;
            let space = page.space()?;
            let bindings = pdfium()?.bindings();

            let annot = unsafe { bindings.FPDFPage_GetAnnot(page.handle, annot_index) };
            if annot.is_null() {
                return Err(PdfError::Pdfium(format!(
                    "page {page_index} has no annotation at index {index}"
                )));
            }
            let read = self.read_annotation(annot, &space);
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };

            read?.ok_or(PdfError::Unsupported(
                "erasing an annotation of a kind this engine does not model",
            ))?
        };

        self.remove_annotation(page_index, index)?;
        Ok(taken)
    }

    fn text_mark_restore(&mut self, page_index: usize, id: i32) -> Result<String> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();

        // Safety: the page is live for the loop and closed by RawPage's drop.
        unsafe {
            for index in 0..bindings.FPDFPage_CountObjects(page.handle) {
                let object = bindings.FPDFPage_GetObject(page.handle, index);
                if object.is_null() || text_mark_id(bindings, object) != Some(id) {
                    continue;
                }
                if let Some(blob) = text_mark_restore_of(bindings, object) {
                    return Ok(blob);
                }
            }
        }

        Err(PdfError::InvalidArgument(format!(
            "page {page_index} has no text mark {id}"
        )))
    }

    /// **Byte-safe, not `FPDFPage_GenerateContent`.** That call rebuilds the
    /// whole content stream from PDFium's own object model — the same
    /// regeneration `set_run_in_stream`'s own doc warns reorders an
    /// untouched paragraph on a real page — and reported from use as undoing
    /// one added text box turning an entire page's own vector artwork
    /// grey: CAMINO's page draws most of its diagram in shapes with their
    /// own colour, and PDFium's regeneration did not carry that colour back
    /// through unchanged. Removing the marked objects by splicing their own
    /// operator spans out of the stream — the same technique
    /// `remove_object`'s own text-run branch already uses for one run at a
    /// time — touches nothing else in the file at all.
    fn remove_text(&mut self, page_index: usize, id: i32) -> Result<()> {
        use crate::pdf::content;

        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        // Which of the page's own objects carry this mark — found through
        // PDFium's own object list, the same walk the old whole-page-
        // regenerating version used, but only to *name* them. Removing them
        // is done byte-safely below.
        let marked: Vec<usize> = {
            let raw = RawPage::open(self.document.handle(), page_number)?;
            let bindings = pdfium()?.bindings();
            let mut found = Vec::new();
            unsafe {
                for index in 0..bindings.FPDFPage_CountObjects(raw.handle) {
                    let object = bindings.FPDFPage_GetObject(raw.handle, index);
                    if !object.is_null() && text_mark_id(bindings, object) == Some(id) {
                        found.push(index as usize);
                    }
                }
            }
            found
        };
        if marked.is_empty() {
            return Err(PdfError::InvalidArgument(format!(
                "page {page_index} has no text mark {id}"
            )));
        }

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);
        let height = self.page_size(page_index)?.height_pt;
        let fonts = self.page_fonts(&file, &page);
        let codes_in = |p: &content::Placed| -> usize {
            let width = p
                .font
                .as_ref()
                .zip(fonts.as_ref())
                .and_then(|(name, dict)| code_width(&file, dict, name))
                .unwrap_or(1)
                .max(1);
            content::pieces(&operations[p.origin.operation])
                .iter()
                .map(|piece| match piece {
                    content::Piece::Codes(bytes) => bytes.len() / width,
                    content::Piece::Kern(_) => 0,
                })
                .sum()
        };

        let mut edits: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
        for object in &marked {
            let Some(run) = self.text_run_at(page_index, *object)? else { continue };
            let order = self.text_order(page_index, *object);
            let (first, last, _) =
                run_operators(&run, height, &placed, &operations, &codes_in, order)?;
            let from = operations[first].span.start;
            let to = operations[last].span.end;
            edits.push((from..to, Vec::new()));
        }
        if edits.is_empty() {
            return Err(PdfError::Unsupported(
                "that text mark cannot be found in the page's own content",
            ));
        }
        let edited = content::splice(&stream, &edits);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    fn add_text_layer(&mut self, page_index: usize, words: &[RecognisedWord]) -> Result<usize> {
        self.validate_page_index(page_index)?;

        // Take off any layer already written under this id, so a second run
        // replaces rather than stacks.
        //
        // Measured, because the obvious check does not show it: writing the
        // same layer three times left `text()` reporting one copy — PDFium's
        // extraction dedupes exactly coincident glyphs — while the saved file
        // grew from 2,023 to 3,037 bytes. The duplicates are real, they are in
        // the file, and they are invisible to the test most likely to be
        // written for this. On a document run through recognition a few times
        // that is unbounded growth and, on any reader whose extraction does not
        // dedupe, doubled search hits.
        let _ = self.remove_text(page_index, TEXT_LAYER_ID);

        let mut written = 0usize;
        for word in words {
            if word.text.trim().is_empty() {
                continue;
            }

            let Some(placed) = word.placement() else { continue };

            // One glyph carrying the whole word. `Glyph::ch` is a `String`
            // precisely because a glyph is "not always one char", and handing
            // the run over whole is what lets PDFium space the letters with the
            // font's own metrics instead of a constant advance that leaves a
            // readable gap after every narrow letter.
            let glyphs = vec![Glyph {
                ch: placed.text.clone(),
                id: 0,
                x: placed.left,
                y: placed.baseline,
                radians: 0.0,
            }];

            // Fully transparent, which is what makes the page look unchanged.
            //
            // The canonical way to do this is text render mode 3, and PDFium
            // exposes `FPDFTextObj_SetTextRenderMode` for it. Alpha zero goes
            // through the text-writing path this crate already has and already
            // tests — real text objects, a marked-content tag, the same
            // coordinate flip — where render mode 3 would mean a second writer
            // for one flag. Both are invisible and both select; this one cannot
            // introduce a class of bug the existing path does not already have.
            let annotation = Annotation::Text {
                text: word.text.clone(),
                font: "Helvetica".into(),
                font_asset: None,
                size: placed.size,
                color: Color { r: 0, g: 0, b: 0, a: 0 },
                glyphs,
                id: TEXT_LAYER_ID,
                restore: String::new(),
                frame: Vec::new(),
                frame_width: 0.0,
            };

            self.add_annotation(page_index, &annotation)?;
            written += 1;
        }
        Ok(written)
    }

    fn extract_pages(&self, range: &[usize]) -> Result<Box<dyn Document>> {
        if range.is_empty() {
            return Err(PdfError::InvalidArgument("no pages to extract".into()));
        }
        for &index in range {
            self.validate_page_index(index)?;
        }

        // Built by copying into a fresh document and reading it back, so the
        // result is an ordinary opened document with no borrow of this one.
        let mut new_doc = pdfium()?
            .create_new_pdf()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        for (position, &source) in range.iter().enumerate() {
            let from = i32::try_from(source).map_err(|_| {
                PdfError::InvalidArgument(format!("page index {source} is out of range"))
            })?;
            let to = i32::try_from(position).unwrap_or(i32::MAX);
            new_doc
                .pages_mut()
                .copy_page_range_from_document(&self.document, from..=from, to)
                .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        }

        let bytes = new_doc
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        Ok(Box::new(PdfiumDocument::open_bytes(bytes, None)?))
    }



    fn save_full_copy(&mut self, dest: &mut dyn Write) -> Result<()> {
        // A document signed and not touched since *is* its signed bytes: a
        // copy of it is those bytes, and rewriting them would break the
        // signature for nothing.
        if self.write_exact_if_unchanged(dest)? {
            return Ok(());
        }
        // A rewrite, and safe to build on the binding's own save: this is the
        // path that is *supposed* to relocate every object. Never the default —
        // it is what destroys a signature's byte range.
        let bytes = if self.remove_password {
            self.without_password()?
        } else {
            self.document
                .save_to_bytes()
                .map_err(|e| PdfError::Pdfium(e.to_string()))?
        };

        // The password goes on last, because everything after it would be
        // writing plaintext into a file that is supposed to be ciphertext.
        let bytes = match &self.security {
            Some(wanted) => self.encrypted(&bytes, wanted)?,
            None => bytes,
        };
        dest.write_all(&bytes)?;
        self.dirty = false;
        // Whatever was kept describes a file that no longer exists. Dropped
        // rather than left to be reported as this document's bytes — a stale
        // copy would have validation vouch for the wrong file.
        self.written = None;
        Ok(())
    }

    /// Append a delta rather than rewriting the file.
    ///
    /// `FPDF_INCREMENTAL` is the whole point: with it the original bytes stay
    /// exactly where they were and a new cross-reference section follows, so any
    /// signature over the original range still verifies. Without it PDFium
    /// renumbers and relocates every object, which breaks every existing
    /// signature irrecoverably — and that is what the binding's own
    /// `save_to_writer` does, since it hardcodes its flags to zero.
    fn save_incremental(&mut self, dest: &mut dyn Write) -> Result<()> {
        // **Guarded here rather than only in the engine**, because this is where
        // every caller ends up — the engine, the JNI bridge, a test. A check one
        // level up protects the callers somebody remembered.
        //
        // Refused rather than quietly upgraded to a full copy: a caller asking
        // for an incremental save is asking to preserve a signature, and
        // silently handing them a file that destroys every signature is its own
        // kind of lie. The message says which save to use instead.
        if self.redacted {
            return Err(PdfError::IncompleteRedaction(
                "a redacted document must be saved as a full copy — saving incrementally \
                 leaves the removed content in the file's earlier revision"
                    .into(),
            ));
        }
        // The same reasoning, and a worse outcome: an appended revision leaves
        // the *entire* original in the file unencrypted, with the secured one
        // after it. A password over that is decoration.
        if self.security.is_some() {
            return Err(PdfError::IncompleteRedaction(
                "a document with a password must be saved as a full copy — saving \
                 incrementally leaves the whole unsecured original in the file"
                    .into(),
            ));
        }
        // And the reverse: a revision appended to an encrypted file is
        // encrypted with it, so the password would still be on.
        if self.remove_password {
            return Err(PdfError::IncompleteRedaction(
                "a document whose password is coming off must be saved as a full copy — \
                 saving incrementally leaves it on"
                    .into(),
            ));
        }

        // The signed bytes themselves, when that is what the document is.
        // Otherwise PDFium's incremental save starts from those same bytes
        // — it was reopened from them — and appends the changes since, so a
        // signature made earlier still covers the revision it was made over.
        if self.write_exact_if_unchanged(dest)? {
            return Ok(());
        }
        let bytes = close_trailing_xref_object(self.save_with_flags(FPDF_INCREMENTAL)?);
        dest.write_all(&bytes)?;
        self.dirty = false;
        // What PDFium just wrote is, by construction, exactly its current
        // page content — including any annotation that was only placed, not
        // yet applied, a moment ago.
        self.exact_content = true;
        // These are the file that now exists, and an incremental save is the
        // one that keeps a signature intact — so they are worth keeping, for
        // exactly the documents where a later check has something to check.
        // A document with no signature keeps nothing: on an 80 MB catalogue
        // this copy is 80 MB, and it would answer no question.
        self.written = (PdfiumDocument::signature_count(self) > 0).then(|| bytes.clone());
        Ok(())
    }

    /// Remove a page, keeping its content so the deletion can be undone.
    ///
    /// The page is copied into a single-page scratch document *before* PDFium is
    /// asked to delete it — afterwards there is nothing left to copy. That
    /// scratch document, serialised, is what [`RemovedPage`] carries.
    fn delete_page(&mut self, index: usize) -> Result<RemovedPage> {
        self.validate_page_index(index)?;
        let page_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;

        let size = self.page_size(index)?;
        let payload = self.page_as_document_bytes(page_index)?;

        unsafe {
            pdfium()?
                .bindings()
                .FPDFPage_Delete(self.document.handle(), page_index);
        }

        self.page_count -= 1;
        self.touch();
        Ok(RemovedPage::new(size, payload))
    }

    /// Put a previously removed page back at `at`.
    fn snapshot_page(&self, index: usize) -> Result<RemovedPage> {
        self.validate_page_index(index)?;
        let page_index = i32::try_from(index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {index} is out of range"))
        })?;
        Ok(RemovedPage::new(
            self.page_size(index)?,
            self.page_as_document_bytes(page_index)?,
        ))
    }

    fn insert_page(&mut self, at: usize, page: RemovedPage) -> Result<()> {
        if at > self.page_count {
            return Err(PdfError::PageOutOfRange {
                index: at,
                count: self.page_count,
            });
        }
        let destination = i32::try_from(at)
            .map_err(|_| PdfError::InvalidArgument(format!("page index {at} is out of range")))?;

        // The payload is a one-page document; importing from it puts the original
        // content back rather than a blank page of the same size.
        let source = pdfium()?
            .load_pdf_from_byte_vec(page.into_payload(), None)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        self.document
            .pages_mut()
            .copy_page_range_from_document(&source, 0..=0, destination)
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;

        self.page_count += 1;
        self.touch();
        Ok(())
    }

    /// Reorder in place. `order[i]` is where the page currently at `i` ends up.
    ///
    /// `FPDF_MovePages` takes the pages to move and a destination, so a whole
    /// permutation is expressed as a sequence of single-page moves: walk the
    /// target order and pull each page to the front of the remaining tail. That
    /// is O(n) moves and, unlike delete-and-reimport, it keeps every page's
    /// objects and annotations intact.
    fn reorder_pages(&mut self, order: &[usize]) -> Result<()> {
        if order.len() != self.page_count {
            return Err(PdfError::InvalidArgument(format!(
                "reorder needs one destination per page: got {} for {} pages",
                order.len(),
                self.page_count,
            )));
        }
        let mut seen = vec![false; order.len()];
        for &to in order {
            if to >= order.len() || seen[to] {
                return Err(PdfError::InvalidArgument(
                    "reorder must be a permutation: every page exactly once".into(),
                ));
            }
            seen[to] = true;
        }

        // `order` says where each page goes; walking the result needs the reverse.
        let mut wanted = vec![0usize; order.len()];
        for (from, &to) in order.iter().enumerate() {
            wanted[to] = from;
        }

        // `current[i]` is the original index of whatever now sits at position i.
        let mut current: Vec<usize> = (0..order.len()).collect();
        let bindings = pdfium()?.bindings();

        for position in 0..wanted.len() {
            let target = wanted[position];
            let at = current
                .iter()
                .position(|&p| p == target)
                .unwrap_or(position);
            if at == position {
                continue;
            }

            let page_index = i32::try_from(at).unwrap_or(i32::MAX);
            let destination = i32::try_from(position).unwrap_or(i32::MAX);
            let ok = unsafe {
                bindings.FPDF_MovePages(self.document.handle(), &page_index, 1, destination)
            };
            if ok == 0 {
                return Err(PdfError::Pdfium(format!(
                    "PDFium refused to move page {at} to {position}"
                )));
            }

            let moved = current.remove(at);
            current.insert(position, moved);
        }

        self.touch();
        Ok(())
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// One sentence, one cause, so a failure says what to do about it.
/// PDF 1.7, the version PDFium writes by default.
const PDF_VERSION_1_7: c_int = 17;

/// `FPDF_INCREMENTAL` from `fpdf_save.h`. Not re-exported by the binding.
const FPDF_INCREMENTAL: u32 = 1;

// ------------------------------------------------------------------ annotation --

/// Annotation subtypes from `fpdf_annot.h`.
///
/// Spelled out because the binding re-exports the *types* but not these
/// constants. They are PDF spec subtype numbers and do not move between PDFium
/// releases.
const ANNOT_TEXT: FPDF_ANNOTATION_SUBTYPE = 1;
const ANNOT_LINK: FPDF_ANNOTATION_SUBTYPE = 2;
const ANNOT_HIGHLIGHT: FPDF_ANNOTATION_SUBTYPE = 9;
/// What `FPDFAction_GetType` returns for a `/URI` action — the only kind of
/// action `read_annotation`'s own `ANNOT_LINK` arm knows how to read back,
/// since it is the only kind `fill_annotation` ever writes.
const PDFACTION_URI: c_ulong = 3;
const ANNOT_UNDERLINE: FPDF_ANNOTATION_SUBTYPE = 10;
const ANNOT_SQUIGGLY: FPDF_ANNOTATION_SUBTYPE = 11;
const ANNOT_STRIKEOUT: FPDF_ANNOTATION_SUBTYPE = 12;
const ANNOT_INK: FPDF_ANNOTATION_SUBTYPE = 15;
const ANNOT_STAMP: FPDF_ANNOTATION_SUBTYPE = 13;

/// `FPDFANNOT_COLORTYPE_Color` — the stroke/foreground colour.
const COLORTYPE_COLOR: FPDFANNOT_COLORTYPE = 0;
/// `FPDFANNOT_COLORTYPE_InteriorColor` — the fill, on the subtypes that have one.
const COLORTYPE_INTERIOR: FPDFANNOT_COLORTYPE = 1;
/// `FPDF_FILLMODE_WINDING`, from `fpdf_edit.h` — not re-exported by the
/// binding any more than the annotation subtypes above are.
const FILLMODE_WINDING: c_int = 2;

/// A page opened straight through the C API, closed when it goes out of scope.
///
/// Deliberately not `pdfium-render`'s `PdfPage`: reaching the raw `FPDF_PAGE` it
/// wraps would need a second patch to the vendored crate, and the annotation
/// calls below are all raw FFI anyway. Opening our own costs one `FPDF_LoadPage`
/// and keeps the vendor diff at the single line it is.
///
/// The `Drop` is the point of the type. Several of the calls below can fail, and
/// an early return that leaked the page would hold a reference to the document
/// for as long as it stayed open — which, since the registry lock is dropped
/// afterwards, would eventually be blamed on something else entirely.
struct RawPage {
    handle: FPDF_PAGE,
}

impl RawPage {
    fn open(document: FPDF_DOCUMENT, index: c_int) -> Result<Self> {
        // Safety: `document` comes from a live `PdfDocument` held by the caller,
        // and `index` has been validated against the page count.
        let handle = unsafe { pdfium()?.bindings().FPDF_LoadPage(document, index) };
        if handle.is_null() {
            return Err(PdfError::Pdfium(format!("could not load page {index}")));
        }
        Ok(RawPage { handle })
    }

    /// The crop-aware mapping for this page.
    ///
    /// Read through the raw boxes rather than `PageSpace::for_page`, which needs a
    /// `PdfPage`: this type deliberately holds only an `FPDF_PAGE`. The answer is
    /// the same one, and it has to be — a mark written against the page height
    /// while text is placed against the crop would land somewhere the text is not.
    fn space(&self) -> Result<PageSpace> {
        let bindings = pdfium()?.bindings();
        let height = unsafe { bindings.FPDF_GetPageHeightF(self.handle) };
        let (mut left, mut bottom, mut right, mut top) = (0.0, 0.0, 0.0, 0.0);

        let read = unsafe {
            bindings.FPDFPage_GetCropBox(self.handle, &mut left, &mut bottom, &mut right, &mut top)
        } != 0
            || unsafe {
                bindings.FPDFPage_GetMediaBox(
                    self.handle,
                    &mut left,
                    &mut bottom,
                    &mut right,
                    &mut top,
                )
            } != 0;

        if !read || !left.is_finite() || !top.is_finite() || right <= left || top <= bottom {
            return Ok(PageSpace::at_origin(height));
        }
        Ok(PageSpace::new(left, top))
    }
}

impl Drop for RawPage {
    fn drop(&mut self) {
        if let Ok(pdfium) = pdfium() {
            unsafe { pdfium.bindings().FPDF_ClosePage(self.handle) };
        }
    }
}

/// One of our rects as PDFium wants it: bottom-left origin, `top` above `bottom`.
///
/// The ordering matters as much as the conversion. Our `Rect` has `top < bottom`
/// because y grows downwards; afterwards `top > bottom`. Handing PDFium an
/// inverted rect produces an annotation with no area, which draws as nothing at
/// all rather than as anything visibly wrong.
///
/// An internal signal only — [`PdfiumDocument::set_runs_in_stream`] returns
/// it, and [`PdfiumDocument::set_text_runs_styled`] catches it and falls
/// back to one call per line; it is never meant to reach a caller outside
/// this file. See `set_runs_in_stream`'s own doc for why it refuses here at
/// all.
const TOO_MANY_EMBEDS_IN_ONE_BATCH: &str =
    "more than one new font embedded in the same batch is not implemented yet";

/// The most a stem can plausibly be, in thousandths of an em. The heaviest face
/// the weights are calibrated on (Montserrat ExtraBold) reads 198; a serif face
/// reads its foot serif too (Times Bold 346, Courier near 380), which says
/// nothing about its weight. Above this a result is "unknown", never a stem.
const MAX_PLAUSIBLE_STEM_MILLI_EM: f32 = 300.0;

/// Letters asked of a font only to learn what it draws for one it does not have:
/// a font that has none of two of them draws the same glyph for both. Capitals
/// and descenders nobody needs for the weight, and rarely all in one subset.
const ABSENT_WITNESSES: [char; 12] = ['Q', 'X', 'Z', 'J', 'K', 'V', 'W', 'Y', 'j', 'k', 'q', 'z'];

/// A glyph's outline as PDFium gives it: every point of its path (control
/// points included), in em — or `None` for no path, or a path with no points.
///
/// **`unicode` is a character, not a glyph number and not a character code**,
/// whatever the header calls the parameter (`glyph`). Measured on three kinds of
/// font: asked for U+0049, a font answers with the glyph that *its own tables*
/// give that character — the `/Encoding` of a simple font, the `/ToUnicode` of a
/// composite one, and then the `CIDToGIDMap` — so a CID subset whose character
/// codes are glyph numbers (Identity-H) still gives its `I`; a code-for-code
/// guess would not. Asked for a character the font has no entry for, it gives
/// the font's `.notdef` (glyph 0) or nothing — see [`glyph_stem_milli_em`].
fn glyph_outline(
    bindings: &dyn PdfiumLibraryBindings,
    font: FPDF_FONT,
    unicode: u32,
) -> Option<Vec<(f32, f32)>> {
    // Asked at size 1000, this PDFium answers in em, not in thousandths of
    // one: Montserrat Light's `I` comes back 0.051 wide.
    let path = unsafe { bindings.FPDFFont_GetGlyphPath(font, unicode, 1000.0) };
    if path.is_null() {
        return None;
    }
    let segments = unsafe { bindings.FPDFGlyphPath_CountGlyphSegments(path) };
    // Control points count, as points of the outline. Exact for the
    // straight-sided `I` and `l`, whose points are their corners.
    let mut points = Vec::new();
    for index in 0..segments.max(0) {
        let segment = unsafe { bindings.FPDFGlyphPath_GetGlyphPathSegment(path, index) };
        let (mut x, mut y) = (0.0f32, 0.0f32);
        if !segment.is_null() && unsafe { bindings.FPDFPathSegment_GetPoint(segment, &mut x, &mut y) } != 0 {
            points.push((x, y));
        }
    }
    (!points.is_empty()).then_some(points)
}

/// Whether two outlines are the very same glyph: as many points, none of them
/// more than a thousandth of an em from its counterpart. Two glyphs of one font
/// that are the same shape *and* the same number of points are the same glyph;
/// there is no tolerance in this for a different letter.
fn same_outline(a: &[(f32, f32)], b: &[(f32, f32)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p.0 - q.0).abs() < 0.001 && (p.1 - q.1).abs() < 0.001)
}

/// How thick a font draws its letters: the ink width of the lower half of the
/// first of `I`, `l` and `i` that it has a real outline for, in thousandths of
/// an em — see [`crate::document::RunStyle::stem_milli_em`] for what that
/// number is and is not. `None` when none of the three can be measured **or the
/// measurement cannot be trusted**.
///
/// **The lower half, so that `i`'s dot does not count.** The dot is wider than
/// the stem it sits on: whole-glyph, `i` read 82 in the datasheet's Light face
/// where `I` reads 51. Measured over every font on its three pages, the lower
/// half of `i` came out exactly the width of the same font's `l`, and for `I`
/// and `l` themselves the lower half is the whole width, so nothing else moves.
///
/// Asked of PDFium's own outline for the glyph rather than of the font file,
/// because that is the one source that answers for every kind of font here — a
/// TrueType subset, a CFF fragment, a composite font with its `CIDToGIDMap` —
/// where a font-file parser stops at the first program that is not an sfnt.
///
/// # What is not a measurement
///
/// **A font that is not embedded.** PDFium draws whatever it found on this
/// machine instead: the datasheet's Light body text, set in a non-embedded
/// `Montserrat-Light`, reads 20 (a Thin) here and 51 beside its own embedded
/// copy, and a non-embedded `Helvetica` reads Arial's 95 — a number about this
/// computer, not about the file. Its name still says what it is.
///
/// **A letter the font has no glyph for, which PDFium answers with `.notdef`.**
/// A subset drops the letters nobody typed, and asked for one of them PDFium
/// gives the font's glyph 0 — a box in some faces (Montserrat's is 0.507 x 0.7
/// em, which read as a stem of 507; ArialNarrow's 0.190 x 0.625, a stem of 190
/// in every invoice that uses it), in others a drawing of its own (Source Sans
/// Pro's reads 476, Calibri's 456). That is what split the paragraphs of a real
/// document whose body was set in two copies of one font, a simple one that
/// carries no `I` (so `l` was measured: 134) and a CID one whose `I` came back
/// `.notdef` (476).
///
/// It is not a fixed shape, so it is found by what it does: **`.notdef` is the
/// one glyph that two different characters draw.** A letter whose outline is
/// the outline of another of the three, or of one of a dozen witnesses no one
/// would mistake for it (`Q`, `X`, `j` …), or of what a character with no entry
/// at all is given (U+E000 and a few more; **not** U+FFFF, which PDFium treats
/// as an invalid character and answers differently), is skipped: the next letter
/// is tried, and a font none of whose three letters it has is unknown. A
/// composite font with no `/ToUnicode`, or none for these letters, is that for
/// every letter: the "cannot be resolved with certainty" case, answered by
/// [`glyph_outline`] going through the font's own tables and finding nothing.
///
/// **A font with no program of its own.** A Type 3 font says it is embedded and
/// has no font file, and PDFium answers outline questions about it from a
/// stand-in (a Chrome-printed page's two Type 3 fonts read Arial's 95).
///
/// **An implausible result.** A glyph with no height, fewer than four points
/// (a stem has four corners), or a stem above
/// [`MAX_PLAUSIBLE_STEM_MILLI_EM`]. That letter is not a measurement; the next
/// one is tried (Liberation Sans Bold's `I` read 549 where its `l` reads 137).
///
/// **Also unknown, as before:** no path at all (a TrueType subset's answer for
/// a letter its embedded copy does not carry: on the datasheet `I` in the
/// SemiBold subsets, `l` in the Regular ones, `l` and `i` in the Medium ones),
/// and a path with no points (a subset can keep a letter's cmap entry and
/// advance width and still have dropped its outline).
fn glyph_stem_milli_em(bindings: &dyn PdfiumLibraryBindings, font: FPDF_FONT) -> Option<u16> {
    // `1` embedded, `0` not, `-1` could not be told.
    if unsafe { bindings.FPDFFont_GetIsEmbedded(font) } != 1 {
        return None;
    }
    // **Embedded, with a program to measure.** A Type 3 font reports itself
    // embedded and has no font program at all — its glyphs are drawings in the
    // page — and PDFium answers outline questions about it from a stand-in: a
    // Chrome-printed page whose two Type 3 fonts read Arial's 95.
    let mut program: usize = 0;
    unsafe { bindings.FPDFFont_GetFontData(font, std::ptr::null_mut(), 0, &mut program) };
    if program == 0 {
        return None;
    }
    // The three letters, asked once. Two of them drawing the very same glyph is
    // a font that has neither (see below), not two measurements.
    let letters = ['I', 'l', 'i'].map(|letter| glyph_outline(bindings, font, letter as u32));
    // What the font draws for a letter it does not have: read the first time a
    // letter gets that far, because most fonts never need it asked.
    let mut absent: Option<Vec<Vec<(f32, f32)>>> = None;
    for (index, points) in letters.iter().enumerate() {
        let Some(points) = points else { continue };
        // The whole box tells a missing glyph from a letter. No point read
        // leaves `left > right`, and so a width below zero.
        let (mut left, mut right) = (f32::MAX, f32::MIN);
        let (mut bottom, mut top) = (f32::MAX, f32::MIN);
        for &(x, y) in points {
            left = left.min(x);
            right = right.max(x);
            bottom = bottom.min(y);
            top = top.max(y);
        }
        let (width, height) = (right - left, top - bottom);
        if points.len() < 4 || !(width > 0.0) || !(height > 0.0) {
            continue;
        }
        // A CFF fragment's `.notdef`, 0.5 x 0.7 em: a box, not a letter.
        if (width - 0.5).abs() < 0.001 && (height - 0.7).abs() < 0.001 {
            continue;
        }
        // The stem is the width of what is left below the middle of the glyph.
        let middle = bottom + height / 2.0;
        let (mut low_left, mut low_right) = (f32::MAX, f32::MIN);
        for &(x, _) in points.iter().filter(|p| p.1 < middle) {
            low_left = low_left.min(x);
            low_right = low_right.max(x);
        }
        let stem = (low_right - low_left) * 1000.0;
        if !(stem > 0.0) || stem > MAX_PLAUSIBLE_STEM_MILLI_EM {
            continue;
        }
        // `.notdef`: the glyph a font draws for a letter it has not got. It is
        // not a fixed shape (a box here, a drawing of its own there), so it is
        // found by what it does: it is the one glyph that **two different
        // characters** draw. Two of the three letters alike, or a letter that
        // looks like a letter nobody would mistake it for (`Q`, `X`, `j` …), or
        // like what a character with no entry at all is given, is that.
        let twin = letters
            .iter()
            .enumerate()
            .any(|(other, q)| other != index && q.as_deref().is_some_and(|q| same_outline(points, q)));
        if twin {
            continue;
        }
        let absent = absent.get_or_insert_with(|| {
            let letters = ABSENT_WITNESSES.iter().map(|c| *c as u32);
            let unmapped = [0xE000u32, 0xFFFD, 0x7F, 0x01];
            letters.chain(unmapped).filter_map(|u| glyph_outline(bindings, font, u)).collect()
        });
        if absent.iter().any(|notdef| same_outline(notdef, points)) {
            continue;
        }
        return Some(stem.round().min(f32::from(u16::MAX)) as u16);
    }
    None
}

impl PdfiumDocument {
    /// The text object at `object` on `page_index`, with the page handle that
    /// keeps it alive.
    ///
    /// **The page must outlive the object.** An `FPDF_PAGEOBJECT` is borrowed
    /// from its page, so returning the object alone would hand back a pointer
    /// into a page PDFium had already closed. The `RawPage` goes back with it
    /// and the caller drops it when finished.
    ///
    /// `None` rather than an error when the object is not text: asking a path
    /// or an image what font it uses has an answer, and the answer is that
    /// there isn't one.
    fn text_object_at(
        &self,
        page_index: usize,
        object: usize,
    ) -> Result<Option<(RawPage, FPDF_PAGEOBJECT)>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();

        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
        let index = i32::try_from(object)
            .map_err(|_| PdfError::InvalidArgument(format!("object {object} is out of range")))?;
        if index < 0 || index >= count {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has no object {object}",
                page_index + 1
            )));
        }

        let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
        if handle.is_null() {
            return Err(PdfError::Pdfium("that object could not be read".into()));
        }
        if unsafe { bindings.FPDFPageObj_GetType(handle) }
            != pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
        {
            return Ok(None);
        }
        Ok(Some((raw, handle)))
    }

    /// Blank one image where it stands, leaving the object in place.
    ///
    /// # Why not remove it
    ///
    /// Removing a page object needs `FPDFPage_GenerateContent`, which re-emits
    /// the **whole** content stream from PDFium's object model rather than
    /// editing the part that changed. Measured on a real catalogue page, that
    /// alone rewrote text nobody had touched — `sky‑light` came back as
    /// `sky -\r\nlight` — and changed how the surrounding paragraph drew. It is
    /// the same hazard `split` documents a few hundred lines up: a page whose
    /// text is re-emitted is a page whose text is no longer quite what it was.
    ///
    /// Replacing the image's pixels changes the object rather than the stream
    /// that references it, so nothing else on the page is rewritten. The image
    /// is as gone as removal would make it — every pixel replaced — and the
    /// vault holds the only copy of what was there.
    ///
    /// One transparent pixel rather than a black rectangle: the object keeps
    /// its matrix, so anything drawn is stretched across the whole frame, and a
    /// lock should leave the page as it would be without the image rather than
    /// stamping a block over its neighbours.
    fn blank_image(&mut self, page_index: usize, object: usize) -> Result<()> {
        let Some((raw, handle)) = self.image_object_at(page_index, object)? else {
            return Err(PdfError::InvalidArgument("that is not an image".into()));
        };
        let bindings = pdfium()?.bindings();

        let clear = [0u8; 4];
        let bitmap = unsafe {
            bindings.FPDFBitmap_CreateEx(
                1,
                1,
                // `FPDFBitmap_BGRA`, which pdfium-render does not re-export.
                // Spelt out rather than reached for through a private module,
                // and pinned by `the_blank_bitmap_format_is_bgra` below.
                BITMAP_BGRA,
                clear.as_ptr() as *mut c_void,
                4,
            )
        };
        if bitmap.is_null() {
            return Err(PdfError::Pdfium("the image could not be cleared".into()));
        }
        let mut pages = [raw.handle];
        let set =
            unsafe { bindings.FPDFImageObj_SetBitmap(pages.as_mut_ptr(), 1, handle, bitmap) };
        unsafe { bindings.FPDFBitmap_Destroy(bitmap) };
        if set == 0 {
            return Err(PdfError::Pdfium("the image could not be cleared".into()));
        }
        // Nothing reaches the file without this, and it re-emits the whole
        // content stream — measured on a catalogue page, that alone rewrote
        // text nobody had touched. Unavoidable for any change to a page object,
        // and survivable only because the way back is the *sealed page* rather
        // than this one: what a lock costs is fidelity while locked, never the
        // original.
        if unsafe { bindings.FPDFPage_GenerateContent(raw.handle) } == 0 {
            return Err(PdfError::Pdfium("the page could not be rewritten".into()));
        }
        Ok(())
    }

    /// The image object at `object`, with the page that keeps it alive — the
    /// image counterpart of [`PdfiumDocument::text_object_at`].
    fn image_object_at(
        &self,
        page_index: usize,
        object: usize,
    ) -> Result<Option<(RawPage, FPDF_PAGEOBJECT)>> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();

        let index = i32::try_from(object)
            .map_err(|_| PdfError::InvalidArgument(format!("object {object} is out of range")))?;
        if index < 0 || index >= unsafe { bindings.FPDFPage_CountObjects(raw.handle) } {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has no object {object}",
                page_index + 1
            )));
        }
        let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
        if handle.is_null() {
            return Err(PdfError::Pdfium("that object could not be read".into()));
        }
        if unsafe { bindings.FPDFPageObj_GetType(handle) }
            != pdfium_render::prelude::FPDF_PAGEOBJ_IMAGE as i32
        {
            return Ok(None);
        }
        Ok(Some((raw, handle)))
    }

    /// Take the words inside a request's shapes off the page, and mark them.
    ///
    /// The content half of a lock, without any of the vault's business — so
    /// that re-applying an area lock after a page has been restored takes the
    /// same path as making one, rather than a second implementation that can
    /// drift from it.
    fn hide_area_content(
        &mut self,
        request: &Redaction,
        catalogue: Option<&crate::document::glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        match self.cut_text_in_area(request) {
            Ok(report) => Ok(report),
            // Shape by shape. PDFium's own path takes one rectangle, and
            // handing it the union of a two-line selection would clear the
            // words either side of what was picked — the very thing the parts
            // exist to avoid.
            Err(_) => {
                let mut report = RedactionReport::default();
                for shape in request.shapes() {
                    let one = Redaction {
                        area: *shape,
                        parts: Vec::new(),
                        ..request.clone()
                    };
                    let part = self.redact_inner(&one, Intent::Apply, catalogue)?;
                    report.characters += part.characters;
                    report.objects += part.objects;
                    report.spilled.extend(part.spilled);
                    report.uncleared.extend(part.uncleared);
                }
                Ok(report)
            }
        }
    }

    /// Take the text inside an area off the page, and mark it.
    ///
    /// **Cuts the operators that drew it out of the content stream**, so every
    /// other byte on the page — the operators drawing the text around it
    /// included — is copied through exactly as it was. See [`crate::pdf`] for
    /// why that matters and what the alternative did.
    ///
    /// # What it removes
    ///
    /// Whole show-text operators, not individual characters. Cutting part of a
    /// string means knowing which bytes are which glyph, and that lives behind
    /// the font's encoding — one byte per glyph in a simple font, two in the
    /// CID fonts a real catalogue uses. So a locked phrase takes the run it sits
    /// in with it, which is usually the line. That is a deliberate trade: it
    /// hides more than was asked for, and it leaves the rest of the page
    /// untouched, which the precise version could not.
    ///
    /// Refuses rather than half-doing it. A run whose operator cannot be found
    /// would be left drawing on the page while the caller was told it had gone.
    fn cut_text_in_area(&mut self, request: &Redaction) -> Result<RedactionReport> {
        use crate::pdf::content;

        let runs = self.text_runs(request.page_index)?;
        let covered: Vec<&crate::document::TextRun> = runs
            .iter()
            .filter(|r| request.touches(&r.rect))
            .collect();
        if covered.is_empty() {
            return Err(PdfError::InvalidArgument("no text in that area".into()));
        }
        let height = self.page_size(request.page_index)?.height_pt;

        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, request.page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;

        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);
        let fonts = self.page_fonts(&file, &page);

        // Every character on the page, with where it sits — what decides
        // precisely which glyphs the selection covers.
        let glyphs = self.page(request.page_index)?.characters()?;
        let per_char = glyphs.boxes.len() / 4;
        // The boxes are one per code unit and the text is UTF-8; where those
        // disagree an index into one does not mean the same thing in the other,
        // so nothing is sliced on that page.
        let indexable = per_char == glyphs.text.chars().count();

        // Every covered run has to be found, or something stays on the page
        // that the caller was told had gone.
        const NEAR: f32 = 4.0;
        // Gathered as it goes, because a sliced run loses only the glyphs the
        // selection covered while one cut whole loses all of them.
        let mut cuts = Cuts::default();

        // Which page glyph begins at a point, if any — used both for where a
        // run starts and for where the operator drawing it starts, whose
        // difference is how far into that operator the run begins.
        let box_at = |i: usize| {
            let b = &glyphs.boxes[i * 4..i * 4 + 4];
            crate::document::Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] }
        };
        let glyph_starting_at = |left: f32, bottom: f32| -> Option<usize> {
            (0..per_char)
                .map(|i| {
                    let r = box_at(i);
                    let d = ((r.left - left).powi(2) + (r.bottom - bottom).powi(2)).sqrt();
                    (i, d)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, d)| *d <= NEAR)
                .map(|(i, _)| i)
        };

        for run in &covered {
            let (want_x, want_y) = (run.rect.left, height - run.rect.bottom);

            // **The operators that draw this run, identified from the
            // stream's own state machine.**
            //
            // PDFium reports a line as one run, but a producer may draw its
            // hyphen — or any fragment — with a `Tj` of its own, so a run is
            // often several operators. Which ones is not a question of
            // proximity: two show-text operators continue each other when
            // nothing between them moved the text down, and `content::placed`
            // reports that as a line number. Grouping by a baseline band
            // instead swept up neighbouring runs and made matching worse —
            // measured, 19 precise and 3 shifted against 27 and 0.
            let start = placed
                .iter()
                .map(|p| {
                    let d = ((p.origin.x - want_x).powi(2) + (p.origin.y - want_y).powi(2)).sqrt();
                    (p, d)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, d)| *d <= NEAR)
                .map(|(p, _)| p);
            let Some(start) = start else {
                return Err(PdfError::Unsupported(
                    "some of that text is drawn in a way this cannot cut out",
                ));
            };

            // Everything on that line, from where the run begins to where it
            // ends. The line number keeps a neighbouring column out; the
            // horizontal bound keeps the rest of a long line out.
            let mut parts: Vec<&content::Placed> = placed
                .iter()
                .filter(|p| {
                    p.line == start.line
                        && p.origin.x >= start.origin.x - 0.5
                        && p.origin.x <= run.rect.right + NEAR
                })
                .collect();
            parts.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));

            // The glyphs the run drew — one per character it reported. Found
            // by where the run starts, nearest rather than
            // within-a-threshold: a run's rectangle bounds its ink while a
            // glyph's box carries its side bearing, so the corners never quite
            // coincide, and a tight threshold found no start at all on fifteen
            // of sixteen real pages.
            let wanted = run.text.chars().count();
            let run_glyph = glyph_starting_at(run.rect.left, run.rect.bottom);
            let mine: Vec<(usize, crate::document::Rect)> = if indexable && wanted > 0 {
                run_glyph
                    .filter(|from| from + wanted <= per_char)
                    .map(|from| (from..from + wanted).map(|i| (i, box_at(i))).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            Self::cut_run(
                &CutContext { file: &file, bytes: &bytes, fonts: fonts.clone(), operations: &operations },
                request,
                &run.text,
                &run.rect,
                &parts,
                mine,
                &[],
                &mut cuts,
            );
        }

        // Anything else starting inside the area — the hyphens and fragments
        // PDFium folds into a neighbouring run rather than reporting
        // separately, which would otherwise be left behind. Only where nothing
        // is being sliced there, or the two edits would fight over the bytes.
        for p in &placed {
            let (x, y) = (p.origin.x, height - p.origin.y);
            if request.holds(x, y)
                && !cuts.edits.iter().any(|(span, _)| *span == operations[p.origin.operation].span)
            {
                cuts.cut_whole.push(p.origin.operation);
            }
        }
        let Cuts { mut edits, mut cut_whole, spilled, characters } = cuts;
        cut_whole.sort_unstable();
        cut_whole.dedup();
        for index in &cut_whole {
            edits.push((operations[*index].span.clone(), Vec::new()));
        }

        let mut edited = content::splice(&stream, &edits);
        if let Some(fill) = request.fill {
            // One mark per shape. Painting the union instead would put a black
            // box over the words a two-line selection deliberately left alone.
            for shape in request.shapes() {
                edited.extend_from_slice(&mark(*shape, height, fill));
            }
        }

        // The edited stream goes into the first of the page's content streams;
        // any others are emptied. Concatenating them was how it was read, so
        // this is the same content, and the page dictionary never changes.
        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(
                b"Filter",
                crate::pdf::Object::Name(b"FlateDecode".to_vec()),
            );
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }

        let rewritten = file.rewrite(&replacements)?;
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        self.touch();
        self.redacted = true;

        Ok(RedactionReport {
            characters,
            objects: edits.len(),
            // What went that was not asked for. A sliced run loses exactly the
            // glyphs the selection covered; one cut whole loses its line, and
            // `spilled` is the field that already exists to say so — a caller
            // told only "locked 42 characters" would not know a line went too.
            spilled,
            uncleared: Vec::new(),
        })
    }

    /// A font's `/ToUnicode` table, if it has a readable one.
fn font_to_unicode(
    file: &crate::pdf::File<'_>,
    bytes: &[u8],
    fonts: &crate::pdf::Dict,
    name: &[u8],
) -> Option<crate::pdf::cmap::ToUnicode> {
    let font = file.resolve(fonts.get(name)?).ok()?;
    let entry = font.as_dict()?.get(b"ToUnicode")?;
    let crate::pdf::Object::Stream(dict, range) = file.resolve(entry).ok()? else {
        return None;
    };
    let decoded = crate::pdf::content::decode(&dict, bytes.get(range)?)?;
    Some(crate::pdf::cmap::parse(&decoded))
}

/// The font dictionary in force for a page, inherited if need be.
    fn page_fonts(
        &self,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
    ) -> Option<crate::pdf::Dict> {
        let resources = self.inherited(file, page, b"Resources").ok().flatten()?;
        let fonts = resources.as_dict()?.get(b"Font")?;
        file.resolve(fonts).ok()?.as_dict().cloned()
    }

    /// Another font already on this page that can spell these words.
    ///
    /// Preferred over writing a new one in: it is a face the document already
    /// uses, so the words look like something on the page rather than like an
    /// intruder, and the file gains nothing. Also how a font this program
    /// embedded on an earlier edit gets reused — it is simply one of the fonts
    /// on the page by then.
    fn borrow_font_on_page(
        &self,
        file: &crate::pdf::File<'_>,
        bytes: &[u8],
        fonts: &crate::pdf::Dict,
        own: &[u8],
        wanted: &str,
    ) -> Option<(Swapped, Vec<u8>)> {
        // **The closest look, not the first font listed.** Several fonts on a
        // page can spell the same words, and taking whichever the resource
        // dictionary lists first put a `600mm` dimension label set in a thin
        // face into the page's ExtraBold heading font. Judged by the weight
        // and slant the names declare; where they say nothing the distances
        // tie and the first listed wins, as it always did.
        let own_look = Self::font_look(file, fonts, own);
        let mut best: Option<(i32, Swapped, Vec<u8>)> = None;
        for (key, _) in fonts.0.iter() {
            if key == own {
                continue;
            }
            let Some(width) = code_width(file, fonts, key) else { continue };
            let (map, ink) = match Self::font_to_unicode(file, bytes, fonts, key) {
                Some(map) => (map, None),
                None if width == 1 => (assumed_ascii(), crate::pdf::subset::drawable_ascii(file, fonts, key)),
                None => continue,
            };
            let reverse = spelling_codes(&map, ink.as_ref(), "");
            if let Some(encoded) = encode_with(&reverse, wanted, width) {
                let (weight, italic) = Self::font_look(file, fonts, key);
                let distance = (weight - own_look.0).abs() + if italic != own_look.1 { 1000 } else { 0 };
                if best.as_ref().map_or(true, |(nearest, ..)| distance < *nearest) {
                    best = Some((
                        distance,
                        Swapped {
                            resource: key.clone(),
                            face: Self::font_display_name(file, fonts, key)
                                .unwrap_or_else(|| format!("/{}", String::from_utf8_lossy(key))),
                            added: Vec::new(),
                            page: None,
                        },
                        encoded,
                    ));
                }
            }
        }
        best.map(|(_, swapped, encoded)| (swapped, encoded))
    }

    /// A font's `/BaseFont`, less the six-letter subset tag a producer puts in
    /// front of it — what to call it to somebody, where `TT3` means nothing.
    fn font_display_name(
        file: &crate::pdf::File<'_>,
        fonts: &crate::pdf::Dict,
        key: &[u8],
    ) -> Option<String> {
        let font = file.resolve(fonts.get(key)?).ok()?;
        let name = String::from_utf8_lossy(font.as_dict()?.get(b"BaseFont")?.as_name()?).to_string();
        let bare = match name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
            _ => &name,
        };
        Some(bare.to_string())
    }

    /// The weight (100 thin … 900 black, 400 when the name says nothing) and
    /// slant a font's `/BaseFont` name declares.
    fn font_look(file: &crate::pdf::File<'_>, fonts: &crate::pdf::Dict, key: &[u8]) -> (i32, bool) {
        let name = fonts
            .get(key)
            .and_then(|font| file.resolve(font).ok())
            .and_then(|font| font.as_dict()?.get(b"BaseFont")?.as_name().map(|n| n.to_vec()))
            .map(|n| String::from_utf8_lossy(&n).to_ascii_lowercase())
            .unwrap_or_default();
        // Longest first: `extrabold` contains `bold`, `semibold` too.
        const WEIGHTS: [(&str, i32); 12] = [
            ("extrabold", 800),
            ("ultrabold", 800),
            ("heavy", 800),
            ("black", 900),
            ("semibold", 600),
            ("demibold", 600),
            ("extralight", 200),
            ("ultralight", 200),
            ("thin", 100),
            ("light", 300),
            ("medium", 500),
            ("bold", 700),
        ];
        let weight = WEIGHTS.iter().find(|(word, _)| name.contains(word)).map_or(400, |(_, w)| *w);
        (weight, name.contains("italic") || name.contains("oblique"))
    }

    /// Write one of the caller's fonts into the document and type with it.
    ///
    /// The last resort, and the only one that changes what the file contains.
    /// See [`crate::pdf::embed`] for what is written; here is where the page is
    /// told the font exists.
    fn embed_typing_font(
        &self,
        file: &crate::pdf::File<'_>,
        page_index: usize,
        page: &crate::pdf::Object,
        wanted: &str,
        reverse: &std::collections::BTreeMap<String, u32>,
        requested: Option<&str>,
    ) -> Result<(Swapped, Vec<u8>)> {
        use crate::pdf::{embed, Object};

        let font_bytes = match requested {
            // A specific font was asked for by name — found or not, spelling
            // or not, that is answered directly rather than falling through
            // to "whichever offered font happens to work".
            Some(face) => {
                let found = self
                    .typing_fonts
                    .iter()
                    .find(|candidate| embed::face_name(candidate).as_deref() == Some(face));
                match found {
                    Some(bytes) if embed::can_spell(bytes, wanted) => bytes,
                    Some(_) => {
                        let offending = wanted
                            .chars()
                            .find(|c| !reverse.contains_key(&c.to_string()))
                            .unwrap_or(' ');
                        return Err(PdfError::InvalidArgument(format!(
                            "{face} cannot spell {offending:?} — pick a different font, or \
                             stay with this one."
                        )));
                    }
                    None => {
                        return Err(PdfError::InvalidArgument(format!(
                            "{face} is not one of the fonts available to write with."
                        )))
                    }
                }
            }
            None => {
                let Some(font_bytes) = self
                    .typing_fonts
                    .iter()
                    .find(|candidate| embed::can_spell(candidate, wanted))
                else {
                    // The message that was there before, plus the way out of it. A
                    // reader who has the document's own typeface can add it.
                    let offending = wanted
                        .chars()
                        .find(|c| !reverse.contains_key(&c.to_string()))
                        .unwrap_or(' ');
                    return Err(PdfError::InvalidArgument(format!(
                        "{offending:?} is not in this text's font. This document embeds only \
                         the characters it already uses, so what can be typed here is: {}. \
                         Add a font with `outlinedfont add <file.ttf>` and it can be written \
                         into the document instead.",
                        typeable(reverse)
                    )));
                };
                font_bytes
            }
        };

        let first = file.next_object_number()?;
        let embedded = embed::truetype(font_bytes, first)?;
        let encoded = embed::win_ansi(wanted).ok_or_else(|| {
            PdfError::InvalidArgument(
                "those characters cannot be written with a Latin encoding".into(),
            )
        })?;

        // A name nothing on the page is using. Numbered rather than fixed
        // because a page can end up with more than one of ours.
        let existing = self.page_fonts(file, page).unwrap_or(crate::pdf::Dict(Vec::new()));
        let mut resource = b"PagifyType".to_vec();
        for suffix in 1..=64u32 {
            let candidate = format!("PagifyType{suffix}").into_bytes();
            if existing.get(&candidate).is_none() {
                resource = candidate;
                break;
            }
        }

        // The page gets its own `/Resources`, copied from whatever it inherits
        // and with the font added. Written onto the page rather than into the
        // dictionary it inherits from, because that one is shared: adding to it
        // would name this font on every page in the document.
        let mut resources = self
            .inherited(file, page, b"Resources")?
            .and_then(|r| r.as_dict().cloned())
            .unwrap_or(crate::pdf::Dict(Vec::new()));
        let mut font_dict = resources
            .get(b"Font")
            .and_then(|f| file.resolve(f).ok())
            .and_then(|f| f.as_dict().cloned())
            .unwrap_or(crate::pdf::Dict(Vec::new()));
        font_dict.set(&resource, Object::Reference(embedded.font, 0));
        resources.set(b"Font", Object::Dict(font_dict));

        let mut page_dict = page
            .as_dict()
            .cloned()
            .ok_or(PdfError::Unsupported("that page cannot be read"))?;
        page_dict.set(b"Resources", Object::Dict(resources));
        let number = self.page_object_number(file, page_index)?;
        let mut page_bytes = Vec::new();
        crate::pdf::write_object(&mut page_bytes, &Object::Dict(page_dict));

        Ok((
            Swapped {
                resource,
                face: embedded.name.clone(),
                added: embedded.objects,
                page: Some((number, page_bytes)),
            },
            encoded,
        ))
    }

    /// What the outlined-type pass sees on a page, and where it loses it.
    ///
    /// Exposed for the same reason [`Self::try_set_run_in_stream`] is: when a
    /// page of drawn words reads back as single letters, the useful question is
    /// which stage dropped them — the shape filter, the clustering, or the
    /// match. Returns, per path object, `(width, height, segments, accepted)`,
    /// then the totals.
    #[allow(clippy::type_complexity)]
    pub fn outlined_report(
        &self,
        page_index: usize,
        catalogue: &crate::document::glyphs::Catalogue,
    ) -> Result<(Vec<(f32, f32, usize, bool)>, usize, usize, usize)> {
        use pdfium_render::prelude::PdfPathSegments;

        let page = self
            .document
            .pages()
            .get(i32::try_from(page_index).unwrap_or(0))
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let (page_width, page_height) = (page.width().value, page.height().value);

        let mut objects = Vec::new();
        for object in page.objects().iter() {
            let Some(path) = object.as_path_object() else { continue };
            let Ok(bounds) = object.bounds() else { continue };
            let width = (bounds.right().value - bounds.left().value).abs();
            let height = (bounds.top().value - bounds.bottom().value).abs();
            let segments = path.segments().len() as usize;
            let accepted = crate::document::classify::looks_like_type(
                width,
                height,
                segments,
                page_width,
                page_height,
            );
            objects.push((width, height, segments, accepted));
        }

        // How the match rate moves with the tolerance, which is the question
        // when the letters are found and not recognised.
        let found = outlined_clusters(&page);
        let clusters = found.len();
        let identified = identify_outlined_glyphs(&page, catalogue);
        let matched = identified.len();
        let accepted = objects.iter().filter(|o| o.3).count();
        Ok((objects, accepted, matched, clusters))
    }

    /// Re-emit a page's content stream and change nothing else.
    ///
    /// Exposed to answer one question: is the damage a *move* does the moving,
    /// or the re-emission that commits it?
    pub fn regenerate_only(&mut self, page_index: usize) -> Result<()> {
        let page_number = i32::try_from(page_index).unwrap_or(0);
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        if unsafe { bindings.FPDFPage_GenerateContent(raw.handle) } == 0 {
            return Err(PdfError::Pdfium("the page could not be rewritten".into()));
        }
        self.touch();
        Ok(())
    }

    /// How far each run is from the nearest operator, measured two ways.
    ///
    /// Exposed for the same reason [`Self::try_set_run_in_stream`] is: the edit
    /// path decides whether it can find a run by comparing origins, and when it
    /// cannot, the useful question is *how far off* and *from what*. Returns
    /// `(object, from the run's box, from the run's own origin)`.
    pub fn run_distances(&self, page_index: usize) -> Result<Vec<(usize, String, f32, f32)>> {
        self.run_distances_indexed(page_index)
            .map(|found| found.into_iter().map(|(o, t, a, b, _, _)| (o, t, a, b)).collect())
    }

    /// The same, plus which operator was nearest and how many there are.
    #[allow(clippy::type_complexity)]
    pub fn run_distances_indexed(
        &self,
        page_index: usize,
    ) -> Result<Vec<(usize, String, f32, f32, usize, usize)>> {
        use crate::pdf::content;

        let runs = self.text_runs(page_index)?;
        let height = self.page_size(page_index)?.height_pt;
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, _) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        let nearest = |x: f32, y: f32| -> (usize, f32) {
            placed
                .iter()
                .enumerate()
                .map(|(at, p)| (at, ((p.origin.x - x).powi(2) + (p.origin.y - y).powi(2)).sqrt()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((0, f32::MAX))
        };
        Ok(runs
            .iter()
            .map(|run| {
                let (at, by_box) = nearest(run.rect.left, height - run.rect.bottom);
                let (_, by_origin) = nearest(run.origin.x, height - run.origin.y);
                (run.object, run.text.clone(), by_box, by_origin, at, placed.len())
            })
            .collect())
    }

    /// Move a picture by editing the page's own bytes.
    ///
    /// # Why this exists beside the other one
    ///
    /// Moving through PDFium's object model has to be committed with
    /// `FPDFPage_GenerateContent`, which re-emits the whole content stream —
    /// and on a real catalogue that came back scrambled. The guard in
    /// [`Self::move_object`] catches it and puts the page back, which is safe
    /// and useless: the picture does not move.
    ///
    /// A picture is drawn by one operator. Wrapping that operator in its own
    /// `q … Q` with a translation moves it and can reach nothing else, because
    /// `Q` puts the graphics state back exactly as it found it. Every other
    /// byte of the page is copied through untouched, which is the guarantee
    /// this crate's whole reader rests on.
    ///
    /// **Matched by order, and only where the page proves order works** — the
    /// same rule the text editor uses. PDFium numbers page objects in the order
    /// the stream draws them, so the *n*th image object is the *n*th image
    /// drawn; that is checked by counting before it is relied on.
    fn move_picture_in_stream(&mut self, page_index: usize, object: usize, by: Point) -> Result<()> {
        // A move is a translation on the page — see `transform_in_stream`,
        // which finds the picture's frame and placeholder and writes just
        // inside them, so the clip and the grey travel with the picture.
        // Page space counts downwards, a content stream upwards.
        self.transform_in_stream(page_index, object, [1.0, 0.0, 0.0, 1.0, by.x, -by.y])
    }

    /// Move one text run by writing where it sits, not by re-emitting the page.
    ///
    /// # What this replaces
    ///
    /// Committing a move through PDFium needs `FPDFPage_GenerateContent`, which
    /// re-emits the whole content stream. On a real catalogue that came back
    /// scrambled — `HSI Lighting I HUE . SATURATION` as
    /// `ABOThe Boer T UOur e xpiunpics` — so [`Self::move_object`] guards it by
    /// reading the page's words before and after and undoing a move that
    /// changed any of them. That guard works, and its cost is that moving text
    /// on exactly the documents worth moving text on refuses: *"moving anything
    /// on this page rewrites its text, so nothing was moved"*.
    ///
    /// # How a run moves without touching anything else
    ///
    /// Glyphs land at `Tm × CTM`, so shifting a run means shifting the text
    /// matrix its operators draw under. Two operators are inserted and **every
    /// other byte of the page is copied through**:
    ///
    /// - before the run's first show-text operator, a `Tm` carrying the matrix
    ///   that was already in force with the move added to its translation;
    /// - after its last, a `Tm` putting the **line** matrix back, so that a
    ///   `Td`, `TD` or `T*` following it starts from where it always did.
    ///
    /// The second one is the half that is easy to miss. `Td` moves relative to
    /// the line matrix rather than the text matrix, and `Tm` sets both — so a
    /// shifted `Tm` with nothing after it drags every following line of the
    /// paragraph along with the run.
    ///
    /// # Where it refuses
    ///
    /// Rather than guess, it declines and lets the caller fall back:
    ///
    /// - a run whose operators cannot be located in the stream;
    /// - operators that are not one contiguous stretch, or that have another
    ///   run's drawing interleaved with them;
    /// - a `BT` or `ET` inside that stretch, which would mean the run is not
    ///   one text object;
    /// - a show-text operator immediately after the run, which continues the
    ///   same line and would need the run's own advance to restore;
    /// - a transform that cannot be inverted, or a matrix with a value that
    ///   cannot be written as a PDF number.
    fn move_run_in_stream(&mut self, page_index: usize, object: usize, by: Point) -> Result<()> {
        use crate::pdf::content;

        let run = self
            .text_run_at(page_index, object)?
            .ok_or_else(|| PdfError::InvalidArgument("that is not a run of text".into()))?;
        let height = self.page_size(page_index)?.height_pt;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);
        let states = content::states(&operations);

        // How many codes each show-text operator draws, which is what says
        // where this run's own operators stop — see `run_operators`.
        let fonts = self.page_fonts(&file, &page);
        let codes_in = |p: &content::Placed| -> usize {
            let width = p
                .font
                .as_ref()
                .zip(fonts.as_ref())
                .and_then(|(name, dict)| code_width(&file, dict, name))
                .unwrap_or(1)
                .max(1);
            content::pieces(&operations[p.origin.operation])
                .iter()
                .map(|piece| match piece {
                    content::Piece::Codes(bytes) => bytes.len() / width,
                    content::Piece::Kern(_) => 0,
                })
                .sum()
        };
        let order = self.text_order(page_index, object);
        let (first, last, continues) =
            run_operators(&run, height, &placed, &operations, &codes_in, order)?;

        // **The move, expressed where the operator lives.** A text matrix's
        // translation reaches the page through the CTM alone, so the distance
        // has to be carried back through it — the same inversion a picture
        // needs, and for the same reason.
        let ctm = states[first].ctm;
        let det = ctm[0] * ctm[3] - ctm[1] * ctm[2];
        if det.abs() < 1e-9 {
            return Err(PdfError::Unsupported(
                "these words are placed by a transform this cannot invert",
            ));
        }
        // Page space counts downwards, a content stream upwards.
        let (wanted_x, wanted_y) = (by.x, -by.y);
        let shift = (
            (ctm[3] * wanted_x - ctm[2] * wanted_y) / det,
            (-ctm[1] * wanted_x + ctm[0] * wanted_y) / det,
        );

        let edits = if let Some((open, _)) = frame_scope(&operations, first..last + 1) {
            // **Its own frame**: a text box holding only this run. The move goes
            // just inside the frame's `q` — outside the text object, where a
            // `cm` is allowed — and carries the box's clip along with the
            // words, which neither matrix strategy below can do.
            let frame_ctm = states[open].ctm;
            let det = frame_ctm[0] * frame_ctm[3] - frame_ctm[1] * frame_ctm[2];
            if det.abs() < 1e-9 {
                return Err(PdfError::Unsupported(
                    "these words are placed by a transform this cannot invert",
                ));
            }
            let local = [
                1.0,
                0.0,
                0.0,
                1.0,
                (frame_ctm[3] * wanted_x - frame_ctm[2] * wanted_y) / det,
                (-frame_ctm[1] * wanted_x + frame_ctm[0] * wanted_y) / det,
            ];
            let at = operations[open].span.end;
            vec![(at..at, concat_matrix(&local)?)]
        } else if continues {
            self.glyph_shift(&stream, &operations, &states, first, last, ctm, (wanted_x, wanted_y))?
        } else {
            // **Nothing goes on drawing this line, so the matrix can simply be
            // set.** A `Tm` before the run puts it where it is going; a second
            // one after puts the *line* matrix back, so a `Td`, `TD` or `T*`
            // that follows starts from where it always did.
            let mut moved = states[first].text;
            moved[4] += shift.0;
            moved[5] += shift.1;
            let restore = states[last].line;
            vec![
                // Written in front of the first operator rather than replacing
                // it, so the operator itself goes through untouched.
                (
                    operations[first].span.start..operations[first].span.start,
                    text_matrix(&moved)?,
                ),
                (
                    operations[last].span.end..operations[last].span.end,
                    text_matrix(&restore)?,
                ),
            ]
        };
        let edited = content::splice(&stream, &edits);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// Resize a run along one or both axes independently: `sy` as a `Tf`
    /// (font size — height, in effect) and `sx` as a `Tz` (horizontal
    /// scale — width, leaving glyph height alone), each written just before
    /// the run's own operators and restored just after — the same "borrow
    /// the state, hand it back" shape [`Self::set_run_in_stream`]'s font
    /// substitution already uses.
    ///
    /// **Why two operators, not one factor.** A single `sqrt(sx·sy)` factor
    /// answered every handle the same way regardless of which one was
    /// dragged — a side handle (`sy` fixed at 1.0) and a corner handle both
    /// changed size, and a top/bottom handle (`sx` fixed at 1.0) changed
    /// size too, when it should have left width alone. Kept separate, a side
    /// handle now stretches width only and a top/bottom handle changes size
    /// only, matching what `Handle::scale` already computes — the bug was
    /// only ever in throwing that distinction away here.
    ///
    /// **The one thing this assumes rather than reads: `Tz` starts at its
    /// PDF default, 100.** `content::placed` does not currently track a
    /// run's horizontal scale the way it tracks font size, so a run whose
    /// producer already set a custom `Tz` gets that value multiplied by
    /// `sx` from a wrong starting point. Reversible with `undo` either way,
    /// and the overwhelming majority of PDFs never touch `Tz` at all.
    ///
    /// **Why this exists next to [`Self::set_text_run_styled`], which also
    /// changes size.** That path goes through PDFium's own
    /// `FPDFPage_GenerateContent` — a full page re-emission — so it has to
    /// re-read every run on the page before and after to prove nothing else
    /// moved, which on a page of a few hundred runs was most of a resize's
    /// wall-clock cost, and on some pages the re-emission really did
    /// scramble something and the whole edit was refused. This splice
    /// cannot scramble anything to check for: everywhere else on the page is
    /// bytes this never touches.
    ///
    /// Takes the run already found, not an object number — `text_runs()`
    /// itself is most of this edit's cost on a page of a few hundred runs
    /// (PDFium's own per-object text extraction, not anything here), so a
    /// caller that has already paid for one full scan for its own reasons
    /// (finding which object was clicked, say) must not be made to pay for
    /// a second just to hand this an index.
    fn resize_run_in_stream(
        &mut self,
        page_index: usize,
        run: &crate::document::TextRun,
        sx: f32,
        sy: f32,
    ) -> Result<()> {
        use crate::pdf::content;

        let height = self.page_size(page_index)?.height_pt;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        let fonts = self.page_fonts(&file, &page);
        let codes_in = |p: &content::Placed| -> usize {
            let width = p
                .font
                .as_ref()
                .zip(fonts.as_ref())
                .and_then(|(name, dict)| code_width(&file, dict, name))
                .unwrap_or(1)
                .max(1);
            content::pieces(&operations[p.origin.operation])
                .iter()
                .map(|piece| match piece {
                    content::Piece::Codes(bytes) => bytes.len() / width,
                    content::Piece::Kern(_) => 0,
                })
                .sum()
        };
        let order = self.text_order(page_index, run.object);
        let (first, last, _) = run_operators(run, height, &placed, &operations, &codes_in, order)?;

        // The font and size already in force where the run starts — written
        // back afterward, unconditionally, because both are graphics state
        // that outlives this run's own operators regardless of whether the
        // very next thing on the page happens to continue the same line.
        let governing = placed
            .iter()
            .find(|p| p.origin.operation == first)
            .ok_or(PdfError::Unsupported("that text selects no font"))?;
        let name = governing
            .font
            .clone()
            .ok_or(PdfError::Unsupported("that text selects no font"))?;
        let name = String::from_utf8_lossy(&name);
        let was_size = governing.size;
        let new_size = (was_size * sy).max(1.0);
        // `Tf` scales a glyph's width and height together, so raising the
        // size to grow a run taller (sy) also grows it wider unless `Tz` is
        // pulled back down by the same amount — this is why a Top/Bottom
        // handle (sx == 1.0, meant to leave width untouched) used to widen
        // the run right along with its height. Scaling from the width
        // already in force (`governing.horizontal_scale`), not an assumed
        // 100%, is what makes two resizes in a row compound correctly
        // instead of the second one erasing the first's stretch.
        let was_tz = content::states(&operations)[first].horizontal_scale * 100.0;
        let new_tz = (was_tz * sx / sy).max(1.0);

        let before_ops = format!("/{name} {new_size} Tf\n{new_tz} Tz\n");
        let after_ops = format!("\n{was_tz} Tz\n/{name} {was_size} Tf");

        let before = operations[first].span.start;
        let after = operations[last].span.end;
        let edits = [
            (before..before, before_ops.into_bytes()),
            (after..after, after_ops.into_bytes()),
        ];
        let edited = content::splice(&stream, &edits);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// Every object on a page: what kind it is and where it sits.
    ///
    /// **What the move guard watches.** Reading the words back caught a
    /// re-emission that scrambled a paragraph, but on a page with no text it
    /// compares an empty list with an empty list and lets anything through —
    /// including a picture PDFium could not decode and so did not write back
    /// out. That is a page object silently disappearing, which is the one
    /// failure a guard exists to prevent.
    fn object_census(&self, page_index: usize) -> Result<Vec<Drawn>> {
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let space = raw.space()?;
        let bindings = pdfium()?.bindings();

        let mut census = Vec::new();
        for index in 0..unsafe { bindings.FPDFPage_CountObjects(raw.handle) } {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null() {
                continue;
            }
            let kind = unsafe { bindings.FPDFPageObj_GetType(handle) };

            // **Colour, because a re-emission can keep every word and every
            // position and still repaint the page.** Reported from use as text
            // changing colour when it was moved, which the words-and-bounds
            // check let straight through.
            let colour = {
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                let fill = unsafe {
                    bindings.FPDFPageObj_GetFillColor(handle, &mut r, &mut g, &mut b, &mut a)
                } != 0;
                let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
                let stroke = unsafe {
                    bindings.FPDFPageObj_GetStrokeColor(
                        handle, &mut sr, &mut sg, &mut sb, &mut sa,
                    )
                } != 0;
                (
                    fill.then_some((r, g, b, a)),
                    stroke.then_some((sr, sg, sb, sa)),
                )
            };

            let (mut l, mut b, mut r, mut t) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            if unsafe { bindings.FPDFPageObj_GetBounds(handle, &mut l, &mut b, &mut r, &mut t) } == 0
            {
                // No measurable bounds is itself a fact worth keeping: one
                // appearing or disappearing is a change.
                census.push(Drawn {
                    kind,
                    rect: Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 },
                    fill: colour.0,
                    stroke: colour.1,
                });
                continue;
            }
            let (left, top) = space.to_top_left(l, t);
            let (right, bottom) = space.to_top_left(r, b);
            census.push(Drawn {
                kind,
                rect: Rect { left, top, right, bottom },
                fill: colour.0,
                stroke: colour.1,
            });
        }
        Ok(census)
    }

    /// Where a page object sits among that page's **paths**, and how many there
    /// are — the ordinal `path_operators` is indexed by.
    fn path_ordinal(&self, page_index: usize, object: usize) -> Result<Option<(usize, usize)>> {
        use pdfium_render::prelude::FPDF_PAGEOBJ_PATH;

        let census = self.object_census(page_index)?;
        let mut which = None;
        let mut seen = 0usize;
        for (index, drawn) in census.iter().enumerate() {
            if drawn.kind as u32 == FPDF_PAGEOBJ_PATH {
                if index == object {
                    which = Some(seen);
                }
                seen += 1;
            }
        }
        Ok(which.map(|w| (w, seen)))
    }

    /// The page objects that are text, in the order PDFium lists them — which is
    /// the order the content stream draws them, one per show-text operator.
    ///
    /// Only each object's kind is asked, one call apiece: [`Self::object_census`]
    /// asks four, and on a drawing of sixty thousand shapes that is the
    /// difference between a delay nobody sees and one they do.
    fn text_objects_in_order(&self, page_index: usize) -> Result<Vec<usize>> {
        let page_number = i32::try_from(page_index).map_err(|_| PdfError::PageOutOfRange {
            index: page_index,
            count: self.page_count,
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let mut text = Vec::new();
        for index in 0..unsafe { bindings.FPDFPage_CountObjects(raw.handle) } {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if !handle.is_null()
                && unsafe { bindings.FPDFPageObj_GetType(handle) }
                    == pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
            {
                text.push(index as usize);
            }
        }
        Ok(text)
    }

    /// Where a page object sits among that page's **text objects**, and how many
    /// there are — the ordinal `content::placed` is indexed by, if the page draws
    /// as many text operators as PDFium found objects. See [`run_operators`].
    ///
    /// `None` rather than an error when it cannot be worked out: the caller
    /// still has the position match to try.
    fn text_order(&self, page_index: usize, object: usize) -> Option<(usize, usize)> {
        let text = self.text_objects_in_order(page_index).ok()?;
        let which = text.iter().position(|o| *o == object)?;
        Some((which, text.len()))
    }

    /// Which names in a page's resources are pictures.
    ///
    /// A `Do` may draw a form rather than an image, and the operator does not
    /// say which — only the resource dictionary does. Shared so that the paths
    /// which move a picture and the one which restacks it agree about what
    /// counts as one.
    fn image_names(
        &self,
        file: &crate::pdf::File,
        page: &crate::pdf::Object,
    ) -> Result<std::collections::BTreeSet<Vec<u8>>> {
        Ok(self
            .inherited(file, page, b"Resources")?
            .and_then(|r| r.as_dict().and_then(|d| d.get(b"XObject")).cloned())
            .and_then(|x| file.resolve(&x).ok())
            .and_then(|x| x.as_dict().cloned())
            .map(|dict| {
                dict.0
                    .iter()
                    .filter(|(_, entry)| {
                        matches!(
                            file.resolve(entry),
                            Ok(crate::pdf::Object::Stream(ref d, _))
                                if d.get(b"Subtype").and_then(crate::pdf::Object::as_name)
                                    == Some(&b"Image"[..])
                        )
                    })
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Where each of a page's own objects is drawn in its stream, as first and
    /// last operation inclusive — or `None` for one this cannot place.
    ///
    /// **What a one-step move needs and a jump to either end does not.** Going
    /// up one means landing just after the neighbour, so the neighbour has to
    /// be found too — whatever kind it is. Each kind is matched the way its own
    /// path already matches it, and each is checked against PDFium's count
    /// before any of its members is trusted.
    fn object_spans(
        &self,
        page_index: usize,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
        operations: &[crate::pdf::content::Operation],
        placed: &[crate::pdf::content::Placed],
    ) -> Result<Vec<Option<(usize, usize)>>> {
        use crate::pdf::{content, Object};
        use pdfium_render::prelude::{
            FPDF_PAGEOBJ_FORM, FPDF_PAGEOBJ_IMAGE, FPDF_PAGEOBJ_PATH, FPDF_PAGEOBJ_SHADING,
            FPDF_PAGEOBJ_TEXT,
        };

        let census = self.object_census(page_index)?;
        let height = self.page_size(page_index)?.height_pt;
        let mut spans: Vec<Option<(usize, usize)>> = vec![None; census.len()];

        // Words, each by its own run.
        let fonts = self.page_fonts(file, page);
        let codes_in = |p: &content::Placed| -> usize {
            let width = p
                .font
                .as_ref()
                .zip(fonts.as_ref())
                .and_then(|(name, dict)| code_width(file, dict, name))
                .unwrap_or(1)
                .max(1);
            content::pieces(&operations[p.origin.operation])
                .iter()
                .map(|piece| match piece {
                    content::Piece::Codes(bytes) => bytes.len() / width,
                    content::Piece::Kern(_) => 0,
                })
                .sum()
        };
        // The page's text objects, listed once for every run below.
        let text_objects = self.text_objects_in_order(page_index).unwrap_or_default();
        for run in self.text_runs(page_index).unwrap_or_default() {
            let order = text_objects
                .iter()
                .position(|o| *o == run.object)
                .map(|which| (which, text_objects.len()));
            if let Ok((first, last, continues)) =
                run_operators(&run, height, placed, operations, &codes_in, order)
            {
                // A run whose line goes on cannot be lifted out — see
                // `restack` — so it is not placed either.
                if !continues {
                    if let Some(slot) = spans.get_mut(run.object) {
                        *slot = Some((first, last));
                    }
                }
            }
        }

        // Everything drawn by an ordinal — the *n*th of its kind in the stream
        // is the *n*th of its kind PDFium reports, once the counts agree.
        let by_kind = |kind: u32, found: Vec<(usize, usize)>, spans: &mut Vec<Option<(usize, usize)>>| {
            let members: Vec<usize> = census
                .iter()
                .enumerate()
                .filter(|(_, d)| d.kind as u32 == kind)
                .map(|(index, _)| index)
                .collect();
            if members.len() != found.len() {
                return;
            }
            for (object, span) in members.into_iter().zip(found) {
                spans[object] = Some(span);
            }
        };

        let names = self.image_names(file, page)?;
        // A picture is its whole unit — placeholder, frame and `Do` — and the
        // placeholder shape belongs to that same unit, so that a step through
        // the order goes over the picture as one thing rather than into it.
        let units: Vec<std::ops::Range<usize>> = image_operators(operations, &names)
            .into_iter()
            .map(|at| picture_unit(operations, at))
            .collect();
        by_kind(
            FPDF_PAGEOBJ_IMAGE,
            units.iter().map(|u| (u.start, u.end - 1)).collect(),
            &mut spans,
        );
        by_kind(
            FPDF_PAGEOBJ_PATH,
            path_operators(operations)
                .into_iter()
                .map(|(r, _)| {
                    match units.iter().find(|u| u.start <= r.start && r.end <= u.end) {
                        Some(unit) => (unit.start, unit.end - 1),
                        None => (r.start, r.end - 1),
                    }
                })
                .collect(),
            &mut spans,
        );

        // Forms are the other thing `Do` draws; shadings are `sh`.
        let forms: Vec<(usize, usize)> = operations
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                op.operator == b"Do"
                    && matches!(op.operands.first(), Some(Object::Name(name)) if !names.contains(name))
            })
            .map(|(i, _)| (i, i))
            .collect();
        by_kind(FPDF_PAGEOBJ_FORM, forms, &mut spans);
        let shadings: Vec<(usize, usize)> = operations
            .iter()
            .enumerate()
            .filter(|(_, op)| op.operator == b"sh")
            .map(|(i, _)| (i, i))
            .collect();
        by_kind(FPDF_PAGEOBJ_SHADING, shadings, &mut spans);

        let _ = FPDF_PAGEOBJ_TEXT;
        Ok(spans)
    }

    /// A block that draws one object exactly as the page did, anywhere in the
    /// stream — the state it was drawn under, every clip that was holding it,
    /// its transform, and the operators themselves, inside one `q`/`Q`.
    ///
    /// **The transforms are chained rather than reset.** `cm` can only
    /// concatenate, so getting from the transform in force at the landing
    /// point to the one a clip was set under, then to the next clip's, then to
    /// the object's, is a sequence of `cm`s each carrying the difference —
    /// which is why every one of them needs the previous to be invertible.
    #[allow(clippy::too_many_arguments)]
    fn carried_block(
        &self,
        stream: &[u8],
        operations: &[crate::pdf::content::Operation],
        states: &[crate::pdf::content::State],
        first: usize,
        last: usize,
        words: bool,
        self_clips: bool,
        landing_ctm: [f32; 6],
    ) -> Result<Vec<u8>> {
        let step = |from: [f32; 6], to: [f32; 6]| -> Result<[f32; 6]> {
            inverse(from).map(|back| matrices(to, back)).ok_or(PdfError::Unsupported(
                "this is placed by a transform this cannot invert",
            ))
        };

        let mut block: Vec<u8> = Vec::new();
        block.extend_from_slice(b"\nq\n");

        // The state it was drawn under — colour, line width, the `gs` that
        // carries transparency, the font — see `state_in_force`.
        for operator in state_in_force(operations, stream, first) {
            block.extend_from_slice(&operator);
            block.push(b'\n');
        }

        // **Every clip that was holding it, replayed.** Each under the transform
        // it was set under, and closed with `n` so it paints nothing — the
        // page already painted whatever that path painted, where it was.
        let mut current = landing_ctm;
        for clip in clips_in_force(operations, states, first) {
            let to_clip = step(current, clip.ctm)?;
            block.extend_from_slice(&concat_matrix(&to_clip)?);
            block.push(b'\n');
            for operation in &operations[clip.construction.clone()] {
                if matches!(operation.operator.as_slice(), b"W" | b"W*") {
                    continue;
                }
                block.extend_from_slice(&stream[operation.span.clone()]);
                block.push(b'\n');
            }
            block.extend_from_slice(if clip.even_odd { b"W* n\n" } else { b"W n\n" });
            current = clip.ctm;
        }

        // Then the object's own transform, as one `cm` from wherever the
        // chain has got to.
        let to_object = step(current, states[first].ctm)?;
        block.extend_from_slice(&concat_matrix(&to_object)?);
        block.push(b'\n');

        if words {
            block.extend_from_slice(b"BT\n");
            block.extend_from_slice(&text_matrix(&states[first].text)?);
            block.push(b'\n');
        }
        for operation in operations.iter().take(last + 1).skip(first) {
            // A shape that also set a clip leaves that job behind — see the
            // caller — so its `W` does not travel.
            if self_clips && matches!(operation.operator.as_slice(), b"W" | b"W*") {
                continue;
            }
            block.extend_from_slice(&stream[operation.span.clone()]);
            block.push(b'\n');
        }
        if words {
            block.extend_from_slice(b"ET\n");
        }
        block.extend_from_slice(b"Q\n");
        Ok(block)
    }

    /// The document's bytes as the editing paths read them, for a probe.
    pub fn readable_bytes_for_probe(&self) -> Result<Vec<u8>> {
        self.readable_bytes()
    }

    /// A page's XObject resources by name, for a probe: name → object number.
    pub fn xobject_names_for_probe(
        &self,
        file: &crate::pdf::File<'_>,
        page_index: usize,
    ) -> Result<Vec<(Vec<u8>, u32)>> {
        let page = self.page_object(file, page_index)?;
        let Some(resources) = self.inherited(file, &page, b"Resources")? else {
            return Ok(Vec::new());
        };
        let Some(xobjects) = resources
            .as_dict()
            .and_then(|d| d.get(b"XObject"))
            .map(|x| file.resolve(x))
            .transpose()?
        else {
            return Ok(Vec::new());
        };
        Ok(xobjects
            .as_dict()
            .map(|d| {
                d.0.iter()
                    .filter_map(|(name, entry)| {
                        entry.as_reference().map(|(number, _)| (name.clone(), number))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Each picture's opacity as the stream sets it — `None` where the stream
    /// cannot be followed, in which case the caller keeps PDFium's answer.
    fn picture_opacities(&self, page_index: usize) -> Option<std::collections::HashMap<usize, f32>> {
        use crate::pdf::{content, Object};
        let pictures = self.images_on(page_index).ok()?;
        let bytes = self.readable_bytes().ok()?;
        let file = crate::pdf::File::parse(&bytes).ok()?;
        let page = self.page_object(&file, page_index).ok()?;
        let (stream, _) = self.page_content(&file, &page).ok()?;
        let operations = content::parse(&stream).ok()?;
        let states = content::states(&operations);
        let names = self.image_names(&file, &page).ok()?;
        let drawn = image_operators(&operations, &names);
        if drawn.len() != pictures.len() {
            return None;
        }
        let ext = self
            .inherited(&file, &page, b"Resources")
            .ok()
            .flatten()
            .and_then(|r| r.as_dict().and_then(|d| d.get(b"ExtGState")).cloned())
            .and_then(|g| file.resolve(&g).ok())
            .and_then(|g| g.as_dict().cloned());
        let mut out = std::collections::HashMap::new();
        for (picture, at) in pictures.iter().zip(drawn) {
            let alpha = states[at]
                .ext_gstate
                .as_ref()
                .and_then(|name| ext.as_ref()?.get(name).cloned())
                .and_then(|g| file.resolve(&g).ok())
                .and_then(|g| g.as_dict().and_then(|d| d.get(b"ca")).cloned())
                .and_then(|ca| match ca {
                    Object::Number(n) => String::from_utf8_lossy(&n).parse::<f32>().ok(),
                    _ => None,
                })
                .unwrap_or(1.0);
            out.insert(picture.object, alpha.clamp(0.0, 1.0));
        }
        Some(out)
    }

    /// A page's content stream, decoded and concatenated, for a probe to read.
    ///
    /// Read-only, and the same bytes the editing paths work on — so a probe
    /// that reports which operator draws a run is reporting about the stream
    /// those paths will edit rather than a re-derived one.
    pub fn page_stream(&self, page_index: usize) -> Result<Vec<u8>> {
        let bytes = self.readable_bytes()?;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, _) = self.page_content(&file, &page)?;
        Ok(stream)
    }

    /// Move a run whose line goes on after it, by moving the **glyphs** and
    /// leaving the pen exactly where it was.
    ///
    /// # The problem this solves
    ///
    /// Where a line is drawn as several show-text operators with nothing
    /// repositioning between them — a brochure setting one phrase in three
    /// fonts does exactly that — the words after a run start wherever its
    /// glyphs left the pen. Shifting the run's text matrix shifts the pen with
    /// it, so everything after it on the line moves too; putting the matrix
    /// back afterwards needs the run's own advance, which is the font's
    /// business and not written anywhere in the stream.
    ///
    /// # Two operators that move glyphs and not the pen
    ///
    /// - **`Ts`, text rise**, offsets where a glyph is painted above the
    ///   baseline and advances nothing at all. That is the vertical.
    /// - **A lone number in a `TJ` array** displaces the pen horizontally
    ///   without drawing, and — unlike `Td` — leaves the *line* matrix alone.
    ///   Put one before the run and its negative after, and the pen ends
    ///   exactly where it would have without either. That is the horizontal.
    ///
    /// Both are exact, and neither needs a glyph width.
    ///
    /// # What it declines
    ///
    /// Anything inside the run that sets the text matrix from the line matrix —
    /// `Td`, `TD`, `T*`, `Tm`, and the `'` and `"` that begin a line before
    /// drawing — because that would discard the horizontal displacement partway
    /// through. Those runs are rare, and the caller falls back.
    #[allow(clippy::too_many_arguments)]
    fn glyph_shift(
        &self,
        stream: &[u8],
        operations: &[crate::pdf::content::Operation],
        states: &[crate::pdf::content::State],
        first: usize,
        last: usize,
        ctm: [f32; 6],
        wanted: (f32, f32),
    ) -> Result<Vec<(std::ops::Range<usize>, Vec<u8>)>> {
        let _ = stream;
        for operation in operations.iter().take(last + 1).skip(first) {
            if matches!(
                operation.operator.as_slice(),
                b"Td" | b"TD" | b"T*" | b"Tm" | b"'" | b"\""
            ) {
                return Err(PdfError::Unsupported(
                    "these words re-position themselves partway through",
                ));
            }
        }

        // **The distance, in the space a rise and a displacement live in.**
        // Both act before the text matrix, so the move has to be carried back
        // through the text matrix *and* the transform — one more step than a
        // matrix translation, which the transform alone reaches.
        let text = states[first].text;
        let full = matrices(text, ctm);
        let Some(back) = inverse(full) else {
            return Err(PdfError::Unsupported(
                "these words are placed by a transform this cannot invert",
            ));
        };
        // A direction, so only the linear part applies.
        let local = (
            wanted.0 * back[0] + wanted.1 * back[2],
            wanted.0 * back[1] + wanted.1 * back[3],
        );

        // The horizontal goes through `TJ`, whose numbers are thousandths of
        // unscaled text space and reach the page multiplied by the font size
        // and the horizontal scaling — so both have to be divided back out.
        let scale = states[first].size * states[first].horizontal_scale;
        let closing = states[last].size * states[last].horizontal_scale;
        if scale.abs() < 1e-6 || closing.abs() < 1e-6 {
            return Err(PdfError::Unsupported(
                "these words are set at a size this cannot displace",
            ));
        }
        let displacement = -local.0 * 1000.0 / scale;
        let putting_back = local.0 * 1000.0 / closing;

        // The vertical is a rise, which is already in unscaled text space.
        let rise = states[first].rise + local.1;

        Ok(vec![
            (
                operations[first].span.start..operations[first].span.start,
                [
                    number_operand(rise)?.as_slice(),
                    b" Ts [",
                    number_operand(displacement)?.as_slice(),
                    b"] TJ",
                ]
                .concat(),
            ),
            (
                operations[last].span.end..operations[last].span.end,
                [
                    b"[".as_slice(),
                    number_operand(putting_back)?.as_slice(),
                    b"] TJ ",
                    number_operand(states[last].rise)?.as_slice(),
                    b" Ts",
                ]
                .concat(),
            ),
        ])
    }

    /// Move one drawn shape by wrapping the operators that paint it.
    ///
    /// **A path is pure graphics**, so `q`/`Q` around it is legal where it is
    /// not inside a text object, and a `cm` in between moves it and reaches
    /// nothing else. The distance is carried back through the transform in
    /// force, exactly as a picture's is, because the path's own coordinates are
    /// written in that space.
    ///
    /// # What it declines
    ///
    /// A path that **also sets a clip** — `W` before its painting operator.
    /// `q`/`Q` saves and restores the clipping path, so wrapping one would put
    /// the clip back the moment it closed and every later thing the clip was
    /// holding in would spill out. Rare, and worth refusing rather than
    /// discovering afterwards.
    fn move_path_in_stream(&mut self, page_index: usize, object: usize, by: Point) -> Result<()> {
        // The same translation, written where the shape's own scope begins.
        self.transform_in_stream(page_index, object, [1.0, 0.0, 0.0, 1.0, by.x, -by.y])
    }

    /// Where an object lives in its page's stream and how to wrap it.
    ///
    /// The one answer every wrapping edit needs — a move, a resize, an
    /// opacity — so that each of them lands in the same place: inside a
    /// picture's frame (with its placeholder), around a shape, around a text
    /// box holding only this run.
    fn object_wrap_site(
        &self,
        page_index: usize,
        object: usize,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
        operations: &[crate::pdf::content::Operation],
        states: &[crate::pdf::content::State],
        placed: &[crate::pdf::content::Placed],
    ) -> Result<WrapSite> {
        use crate::pdf::content;
        let pictures = self.images_on(page_index)?;
        if let Some(which) = pictures.iter().position(|i| i.object == object) {
            let names = self.image_names(file, page)?;
            let drawn = image_operators(operations, &names);
            if drawn.len() != pictures.len() {
                return Err(PdfError::Unsupported(
                    "this page draws its pictures in a way this cannot follow",
                ));
            }
            let at = drawn[which];
            return Ok(match frame_scope(operations, at..at + 1) {
                Some((open, close)) => {
                    let start = placeholder_before(operations, open);
                    match start {
                        // Placeholder and frame together need a scope of their own.
                        Some(first) => WrapSite {
                            span: first..close + 1,
                            own_scope: false,
                            ctm: states[open].ctm,
                        },
                        // The frame is a scope of its own: write just inside it.
                        None => WrapSite { span: open..close + 1, own_scope: true, ctm: states[open].ctm },
                    }
                }
                None => WrapSite { span: at..at + 1, own_scope: false, ctm: states[at].ctm },
            });
        }

        if let Some((which, paths)) = self.path_ordinal(page_index, object)? {
            let painted = path_operators(operations);
            if painted.len() != paths {
                return Err(PdfError::Unsupported(
                    "this page paints its shapes in a way this cannot follow",
                ));
            }
            let (span, clips) = painted
                .get(which)
                .cloned()
                .ok_or(PdfError::Unsupported("that shape is not painted on this page"))?;
            if clips {
                return Err(PdfError::Unsupported(
                    "this shape also sets a clipping path, which cannot move with it",
                ));
            }
            return Ok(match frame_scope(operations, span.clone()) {
                Some((open, close)) => WrapSite { span: open..close + 1, own_scope: true, ctm: states[open].ctm },
                None => WrapSite { span: span.clone(), own_scope: false, ctm: states[span.start].ctm },
            });
        }

        let runs = self.text_runs(page_index)?;
        if let Some(run) = runs.iter().find(|r| r.object == object) {
            let height = self.page_size(page_index)?.height_pt;
            let fonts = self.page_fonts(file, page);
            let codes_in = |p: &content::Placed| -> usize {
                let width = p
                    .font
                    .as_ref()
                    .zip(fonts.as_ref())
                    .and_then(|(name, dict)| code_width(file, dict, name))
                    .unwrap_or(1)
                    .max(1);
                content::pieces(&operations[p.origin.operation])
                    .iter()
                    .map(|piece| match piece {
                        content::Piece::Codes(bytes) => bytes.len() / width,
                        content::Piece::Kern(_) => 0,
                    })
                    .sum()
            };
            let order = self.text_order(page_index, object);
            let (first, last, _) = run_operators(run, height, placed, operations, &codes_in, order)?;
            return match frame_scope(operations, first..last + 1) {
                Some((open, close)) => Ok(WrapSite { span: open..close + 1, own_scope: true, ctm: states[open].ctm }),
                // **Worded to survive `Unsupported`'s own template.** Its
                // `Display` always appends "is not implemented yet" — fine
                // for a short phrase, but the previous wording here ended
                // with "...which Edit Text changes", and with that suffix
                // glued on read as "Edit Text changes is not implemented
                // yet" — the opposite of true: Edit Text's own size field
                // already does this, through a completely different,
                // PDFium-object path that does not care whether a run has
                // its own scope. Reported from use as "it says the fix
                // doesn't work either". This version's last clause is about
                // the *handle* specifically, so the suffix lands on
                // something that really is unbuilt.
                None => Err(PdfError::Unsupported(
                    "a handle cannot resize these words — they share a text box with \
                     others, so dragging only moves the whole box. Edit Text already \
                     changes their actual size directly; doing that by dragging a \
                     handle here",
                )),
            };
        }

        Err(PdfError::Unsupported("that is not something this can transform"))
    }

    /// Apply an affine transform, given in page space, to one object — by
    /// writing a single `cm` where its own scope begins.
    ///
    /// Everything drawn by the object goes through its scope's transform to
    /// reach the page, so a change wanted *on the page* is carried back
    /// through that transform: with `C` in force, `C · M · C⁻¹` written inside
    /// makes the page see `M`. A move is `M` = a translation; a resize about a
    /// point is a scale conjugated by that point.
    fn transform_in_stream(&mut self, page_index: usize, object: usize, page_matrix: [f32; 6]) -> Result<()> {
        use crate::pdf::content;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let states = content::states(&operations);
        let placed = content::placed(&operations);

        let site = self.object_wrap_site(page_index, object, &file, &page, &operations, &states, &placed)?;
        let back = inverse(site.ctm).ok_or(PdfError::Unsupported(
            "this is placed by a transform this cannot invert",
        ))?;
        let local = matrices(matrices(site.ctm, page_matrix), back);

        let from = operations[site.span.start].span.start;
        let to = operations[site.span.end - 1].span.end;
        let edited = if site.own_scope {
            // Just inside the scope's `q`.
            let at = operations[site.span.start].span.end;
            content::splice(&stream, &[(at..at, concat_matrix(&local)?)])
        } else {
            let mut opening: Vec<u8> = b"q\n".to_vec();
            opening.extend_from_slice(&concat_matrix(&local)?);
            content::splice(&stream, &[(from..from, opening), (to..to, b"\nQ".to_vec())])
        };

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }
        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// The byte-safe move, exposed so a probe can ask *why* it refused.
    ///
    /// The ordinary path swallows the reason and falls back to PDFium; knowing
    /// what it swallowed is how the coverage gets better — the same bargain as
    /// [`Self::try_set_run_in_stream`].
    pub fn try_move_run_in_stream(
        &mut self,
        page_index: usize,
        object: usize,
        by: Point,
    ) -> Result<()> {
        self.move_run_in_stream(page_index, object, by)
    }

    /// The content-stream edit, exposed so a probe can ask *why* it refused.
    ///
    /// The ordinary path swallows the reason and falls back to PDFium; knowing
    /// what it swallowed is how the coverage gets better.
    pub fn try_set_run_in_stream(
        &mut self,
        page_index: usize,
        object: usize,
        text: &str,
    ) -> Result<()> {
        self.set_run_in_stream(page_index, object, text, None, None).map(|_| ())
    }

    /// Change one run's words, and optionally its size, by editing the
    /// content stream.
    ///
    /// Returns what the run said before, and the size it was drawn at
    /// before this call — the caller's own "was", for undo, since this never
    /// touches anything else in the stream that would carry the old size
    /// forward on its own the way an untouched page's own `Tf` would.
    ///
    /// Refuses — leaving the page untouched — wherever it cannot be certain:
    /// a filter it cannot read, a run whose operator cannot be located, a
    /// font with no `/ToUnicode` to encode the new text through, or a
    /// character that font cannot spell. The caller then falls back to
    /// PDFium, which always works and costs the page's layout.
    ///
    /// `requested_face` names one of `self.typing_fonts` by
    /// [`crate::pdf::embed::face_name`] — `None` runs the automatic three-tier
    /// fallback below; `Some` skips straight to that font (embedding it if
    /// it is not already on the page) and refuses, rather than silently
    /// falling back to another face, if it cannot spell the text.
    ///
    /// `new_size` — `None` keeps drawing at the size this run already used;
    /// `Some` is written into the very `Tf` this run's own text already
    /// needs in front of it (see [`content::replacing_codes`]), so a size
    /// change costs nothing beyond what a words-only edit already pays and
    /// cannot touch any other run's own `Tf` the way asking PDFium to
    /// regenerate the whole page can.
    fn set_run_in_stream(
        &mut self,
        page_index: usize,
        object: usize,
        text: &str,
        requested_face: Option<&str>,
        new_size: Option<f32>,
    ) -> Result<(String, f32)> {
        use crate::pdf::content;

        // **One run, not every run.** This used to be `text_runs_all()`
        // filtered down to the one object wanted — on a page of a few
        // hundred runs, most of a second spent extracting the words of
        // every *other* run just to read this one's own origin.
        // `text_runs_some` answers the same question for one object without
        // touching the rest. `run_list` itself is only ever needed below, in
        // the rare fallback where neither the page's order nor geometry could
        // place this run among the page's content stream operations —
        // fetched there, lazily, not here.
        //
        // **Not `text_run_at`, which leaves out a run with no ink.** A lone
        // `l` or `-` under half a point wide is not something to click, but it
        // is a text object with a place in the page's order like any other,
        // and a line with one on it must not be refused for it.
        let run = self
            .text_runs_some(page_index, &std::collections::HashSet::from([object]))?
            .into_iter()
            .next()
            .ok_or_else(|| PdfError::InvalidArgument("that is not a text run".into()))?;
        // Where this object falls among the page's text objects, counting every
        // one: the operator that drew it is the same number among the stream's
        // show-text operators, if the stream bears that out — see below.
        let order = self.text_order(page_index, object);
        let previous = run.text.clone();
        let height = self.page_size(page_index)?.height_pt;
        self.substituted = None;

        // Readable by *this* crate, which an encrypted document is not until
        // its security comes off — see `readable_bytes`.
        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let fonts = self.page_fonts(&file, &page);

        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        // **The baseline origin, not the box's corner** — see `run_operators`'s
        // own doc for the exact same fix, needed there first: the box's
        // bottom sits a descender below the baseline, a gap that grows with
        // the font's own size, so probing from `rect.bottom` missed the
        // right operation entirely the moment a size change made that gap
        // wide enough — reported from use as a size change coming back
        // unable to be undone, on the run it had just resized.
        let (want_x, want_y) = (run.origin.x, height - run.origin.y);
        // **By count first, where the stream bears the count out.** The *n*th
        // text object is the *n*th show-text operator — see `placed_by_order` —
        // which finds a piece that continues the one before it, and does not
        // mistake it for that one, as a match by position does when the piece
        // before it is narrow. The position match is for a page whose count the
        // stream does not confirm.
        let codes_in = |p: &content::Placed| codes_drawn(&file, fonts.as_ref(), &operations, p);
        let found = match placed_by_order(&run, height, &placed, order, &codes_in)
            .or_else(|| nearest_placed(&placed, want_x, want_y).map(|(at, _)| &placed[at]))
        {
            Some(found) => found,
            // **Where it is drawn is not always where the walk thinks.**
            //
            // The text matrix advances by the width of whatever was just shown,
            // and working that out needs the font's glyph widths — which the
            // stream walk does not have. So a second `Tj` on the same line,
            // with nothing repositioning between them, is reported back at the
            // first one's origin. Measured on a real page: twenty-five runs
            // placed to within two points and four out by fifteen to sixty-two,
            // every one of the four a continuation of the run before it.
            //
            // Counting is the way out — but only where the page has *shown*
            // that counting works. See `placed_in_order`.
            None => {
                // The rare path: geometry alone did not place this run, so
                // every run's own position is needed to count where it falls
                // among the page's content-stream operations — see
                // `placed_in_order`. Paid here, not for every ordinary edit
                // that `nearest_placed` above already resolves directly.
                let run_list = self.text_runs_all(page_index)?;
                let index = run_list
                    .iter()
                    .position(|r| r.object == object)
                    .ok_or(PdfError::Unsupported("that is not a text run"))?;
                placed_in_order(&run_list, &placed, height, index).ok_or(PdfError::Unsupported(
                    "that text is drawn in a way this cannot edit",
                ))?
            }
        };

        let name = found
            .font
            .clone()
            .ok_or(PdfError::Unsupported("that text selects no font"))?;
        let fonts = fonts.ok_or(PdfError::Unsupported("that page declares no fonts"))?;
        let width = code_width(&file, &fonts, &name)
            .ok_or(PdfError::Unsupported("that font's codes cannot be counted"))?;
        // A font's own table where it has one. Where it has none — plenty of
        // simple fonts do not — a **single-byte** font is read as ASCII, which
        // is what the standard encodings all agree on for the printable range.
        // Checked rather than assumed: the mapping is only used for characters
        // below 128, and anything else still refuses.
        //
        // **A code is not a glyph.** The ASCII reading says what each code
        // *means*; a subset keeps only the outlines it drew, and a code with
        // none draws an empty box with no error anywhere. `ink` is what the
        // font's own `/CharSet` says survived — see [`crate::pdf::subset`].
        let mut ink = None;
        let unicode = match Self::font_to_unicode(&file, &bytes, &fonts, &name) {
            Some(map) => map,
            None if width == 1 => {
                ink = crate::pdf::subset::drawable_ascii(&file, &fonts, &name);
                assumed_ascii()
            }
            None => {
                return Err(PdfError::Unsupported(
                    "that font carries no character map this can read",
                ))
            }
        };

        // The map read backwards: what code spells each character. Where two
        // codes spell the same thing the lower one wins, which keeps the choice
        // stable rather than dependent on iteration order. `unicode` itself is
        // left whole: it is what the run's *existing* codes are lined up
        // against, and those were drawn, whatever a `/CharSet` says.
        let reverse = spelling_codes(&unicode, ink.as_ref(), &run.text);

        // Encode the new words. A character this font cannot spell would come
        // out as a blank or the wrong glyph, so it refuses instead.
        //
        // **Trailing whitespace is dropped rather than refused.** PDFium
        // appends a space of its own to a run's extracted text, and plenty of
        // fonts have no space glyph at all — spaces are made by moving the pen,
        // not by drawing. Between them that made a run impossible to retype
        // *with its own words*: measured on a real document, four runs of
        // twenty-nine. A trailing space draws nothing, so losing it costs
        // nothing.
        let wanted = encodable(text, &reverse);

        // **Three fonts to try, in the order that keeps the page looking like
        // itself.**
        //
        // Its own font first: the words then draw exactly as their neighbours
        // do. Failing that, another font already on this page — a different
        // face, but one the document already uses, and no new bytes in the
        // file. Failing that, a font the caller offered, written into the
        // document.
        //
        // Only the last changes what the file contains, and both of the last
        // two change how the words look — which the caller is told about
        // rather than left to notice.
        // **A requested face skips the fallback order entirely.** Someone who
        // opened the font picker asked for a specific typeface, not "whatever
        // already spells this" — keeping the current font because it happens
        // to cover these characters, or borrowing an unrelated one already on
        // the page, would both silently ignore the choice just made.
        let (encoded, swap) = if let Some(face) = requested_face {
            let (swap, encoded) =
                self.embed_typing_font(&file, page_index, &page, &wanted, &reverse, Some(face))?;
            (encoded, Some(swap))
        } else {
            match encode_with(&reverse, &wanted, width) {
                Some(bytes) => (bytes, None),
                None => match self.borrow_font_on_page(&file, &bytes, &fonts, &name, &wanted) {
                    Some((swap, encoded)) => (encoded, Some(swap)),
                    None => {
                        let (swap, encoded) =
                            self.embed_typing_font(&file, page_index, &page, &wanted, &reverse, None)?;
                        (encoded, Some(swap))
                    }
                },
            }
        };

        // Which of the operator's codes this run occupies.
        let operation = &operations[found.origin.operation];
        let codes = codes_of(operation, width);
        let spellings: Vec<Option<String>> =
            codes.iter().map(|code| unicode.get(code).cloned()).collect();
        let owner = align_codes(&spellings, &run.text)
            .or_else(|| (codes.len() == run.text.chars().count()).then(|| (0..codes.len()).collect()))
            .ok_or(PdfError::Unsupported("that run's codes cannot be lined up with its text"))?;
        let span = owned_codes(&owner).ok_or(PdfError::Unsupported(
            "that run's characters belong to none of its codes",
        ))?;
        let (first, last) = (span.start, span.end - 1);

        // The name to select, and — deliberately — the *original* code width,
        // which is what walks the operator's existing codes. The replacement
        // bytes are already encoded in whatever font is drawing them.
        let drawing = swap.as_ref().map(|s| s.resource.clone()).unwrap_or_else(|| name.clone());
        // **`new_size`, like every size this crate hands a caller, is the
        // effective size — `run.size`, not `found.size`.** `found.size` is
        // the raw `Tf` value this content-stream parse itself found, which
        // is the one thing `Tf` can actually hold; `run.size` is the same
        // run read through `text_runs`, which folds in whatever stretch the
        // text matrix carries (see `set_text_run_styled`'s own `vertical_
        // scale` doc). `run.size / found.size` is that stretch, recovered
        // without a second matrix read, and dividing the caller's request by
        // it is what turns "draw this run at 25pt" back into the raw number
        // `Tf` needs to actually produce 25pt.
        let vertical_scale = if found.size.abs() > 1e-6 { run.size / found.size } else { 1.0 };
        // Zero or negative asks nothing sensible of `Tf` and is treated the
        // same as "unchanged" — the same guard the slower, PDFium-driven
        // path already applies to a requested size.
        let size = new_size
            .filter(|s| *s > 0.0)
            .map(|wanted| if vertical_scale.abs() > 1e-6 { wanted / vertical_scale } else { wanted })
            .unwrap_or(found.size);
        let mut replacement = content::replacing_codes(
            operation,
            first..last + 1,
            &encoded,
            &drawing,
            size,
            width,
        );
        // **Put the page's own font — and size — back.** `Tf` is graphics state:
        // it stays selected until something changes it, so anything drawn after
        // this that relies on the one in force would come out in a face, or at a
        // size, nobody asked for. A size is no less a `Tf` than a face is.
        if swap.is_some() || size != found.size {
            replacement.extend_from_slice(
                format!("\n/{} {} Tf", String::from_utf8_lossy(&name), found.size).as_bytes(),
            );
        }
        let edited = content::splice(&stream, &[(operation.span.clone(), replacement)]);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }

        // The page, where it had to be told about a font it did not have.
        let mut extra: Vec<(u32, Vec<u8>)> = Vec::new();
        if let Some(swap) = &swap {
            extra.extend(swap.added.iter().cloned());
            if let Some(page) = &swap.page {
                replacements.push(page.clone());
            }
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &extra)?;
        // Recorded rather than returned, so the caller can say what happened
        // without every signature in the chain growing a field for it.
        let face = swap.map(|s| s.face);
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)?;
        self.substituted = face;
        // The effective size, matching every other size this crate hands a
        // caller — not `found.size`, the raw `Tf` value, for the same reason
        // `was.size` above is not the slower path's own raw read either.
        Ok((previous, run.size))
    }

    /// Several runs on the same page, written in one pass — the batched
    /// counterpart to [`Self::set_run_in_stream`], for a multi-line
    /// paragraph's several lines sharing one transaction instead of one
    /// each.
    ///
    /// **Why this exists.** Every call to `set_run_in_stream` independently
    /// pays `edit_base()` (`save_to_bytes()` — a full PDFium
    /// re-serialisation of the whole document, unless it is signed) and
    /// `adopt_edit()` (a full PDFium reopen of the rewritten bytes), on top
    /// of parsing the page's own content stream. None of that depends on
    /// which run is being edited, so paying it once per line of a paragraph
    /// — which `apply_paragraph_edit` in the app does, one `SetTextRun` per
    /// line — multiplies a cost that has nothing to do with how many lines
    /// changed. Reported from use: a three-line paragraph took close to a
    /// second to apply, after `set_run_in_stream`'s own per-run identity
    /// lookup had already been fixed — the remaining cost scaled with line
    /// count, which only this, not that earlier fix, explains.
    ///
    /// Parses the page's content stream once, finds and encodes every
    /// requested run's own replacement against that one parse, splices every
    /// span in in one pass (`content::splice` already takes a slice of
    /// them), and writes/reopens the result once for the whole group.
    ///
    /// **Each run's operator is found by count where the stream bears the count
    /// out** — the *n*th text object is the *n*th show-text operator, held to
    /// the stream by [`order_agrees`] — and only otherwise by position. That is
    /// what lets a line drawn as several pieces be retyped, or hidden, piece by
    /// piece: a piece that continues the one before it has no position of its
    /// own to be found by. Refused whole, with nothing written, when two of the
    /// runs resolve to one operator or one of them cannot be resolved at all.
    ///
    /// **Refuses, deliberately, the moment a second edit in the group would
    /// also need a brand new font embedded.** `embed_typing_font` picks a
    /// resource name and a page-resources patch by looking at what the page
    /// (and this crate's own insertions project onto it) already has — fine
    /// once, but two calls made here against the same unmodified starting
    /// point would each pick the *same* free name and each produce its own
    /// complete resources patch, the second silently discarding the first's
    /// addition. Rather than get a shared allocation table right for a case
    /// this rare (two different lines of one paragraph each spelling
    /// characters nothing already on the page covers), this refuses with
    /// [`TOO_MANY_EMBEDS_IN_ONE_BATCH`] and leaves [`Self::set_text_runs_styled`]
    /// to fall back to one call per line — correct, just not faster, the
    /// same as before this existed.
    ///
    /// **A `requested_color` entry is a colour-only change, not a retype.**
    /// `apply_paragraph_edit`'s own `hidden_or_written` recolours a shrinking
    /// paragraph's now-unwanted lines to the page's background instead of
    /// blanking them (see that function's own doc), which is a completely
    /// ordinary part of editing a paragraph that already has more than one
    /// run per line — not a rare shape. Reported from use: with this gate
    /// absent, a `requested_color` edit mixed into an otherwise ordinary
    /// batch failed `set_text_runs_styled`'s own fast-path check (it is not
    /// `TextStyle::default()`), silently sending the *whole* paragraph back
    /// through one call per line — exactly the cost this function exists to
    /// avoid, on exactly the documents where lines split across several
    /// runs are common enough that nearly every real paragraph hit it.
    ///
    /// **`removals` are objects to take off the page in the same pass**, each
    /// found the way an edit's run is and cut out whole — see
    /// [`DocumentMut::replace_text_lines`], which is what asks for it. With none,
    /// this is exactly what it always was.
    ///
    /// **`stretch` names retyped runs that are to span a width** (points, from the
    /// run's left origin): each such run is written with its gaps opened or closed
    /// by `TJ` spacing numbers until PDFium's own box for it is that wide — see
    /// [`Stretch`]. The width is measured, not worked out from glyph advances: the
    /// batch is first written *plain* to a copy of the document that is read back
    /// and thrown away, and only the stretched version reaches `self`, so a line
    /// that cannot be stretched is refused with nothing written. With none, this
    /// is exactly what it always was.
    fn set_runs_in_stream(
        &mut self,
        page_index: usize,
        edits: &[(usize, &str, Option<&str>, Option<f32>, Option<crate::document::Color>)],
        removals: &[usize],
        stretch: &HashMap<usize, f32>,
    ) -> Result<Vec<(String, crate::document::TextStyle)>> {
        use crate::pdf::content;

        // **Temporary — see `last_batch_timing`'s own doc.** A local `Vec`
        // throughout, assigned to `self` only once at the end, so pushing
        // into it never fights the `&self`/`&mut self` calls this function
        // already makes in between.
        let mut timing: Vec<(&'static str, std::time::Duration)> = Vec::new();
        let t = std::time::Instant::now();

        self.substituted = None;
        let height = self.page_size(page_index)?.height_pt;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        timing.push(("edit_base", t.elapsed()));

        let t = std::time::Instant::now();
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let fonts = self.page_fonts(&file, &page);

        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);
        // The state each operator draws in (horizontal scaling is read from it),
        // walked only when a line is to be stretched.
        let states = if stretch.is_empty() { Vec::new() } else { content::states(&operations) };
        timing.push(("parse_file_and_stream", t.elapsed()));

        let mut spans: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
        let mut extra: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut page_patch: Option<(u32, Vec<u8>)> = None;
        let mut embeds = 0usize;
        let mut face_written: Option<String> = None;
        let mut results = Vec::with_capacity(edits.len());
        // The lines to be spread to a width, and what it takes to write each again.
        let mut stretches: Vec<Stretch> = Vec::new();
        let t_loop = std::time::Instant::now();

        // **Every requested run, resolved against one page open.** Asking
        // `text_run_at` once per edit opened the page, loaded its text layer
        // and walked to the one object afresh for each — about 9 ms a line on
        // the datasheet, which was three quarters of a 33-line apply (305 of
        // 415 ms) and everything per line that the apply cost.
        // `text_runs_some` does the page once for all of them and gives each
        // the run `text_run_at` would (`tests/text_words.rs` checks that field
        // by field) — **and keeps the ones with no ink area, which
        // `text_run_at` drops.** A lone `l` or `-` under half a point wide is
        // not a run to click but is a text object to edit, with a place in the
        // page's order like any other, and a paragraph with one on a line must
        // not be refused for it. Only an object that is not text at all is
        // "not a text run", and that is refused before anything is written.
        let t_resolve = std::time::Instant::now();
        let wanted: std::collections::HashSet<usize> =
            edits.iter().map(|e| e.0).chain(removals.iter().copied()).collect();
        let resolved: HashMap<usize, crate::document::TextRun> = self
            .text_runs_some(page_index, &wanted)?
            .into_iter()
            .map(|run| (run.object, run))
            .collect();
        // Every text object of the page, once for the whole batch, in the order
        // the page draws them. Where a requested object falls among them is the
        // number of the show-text operator that drew it, if the stream bears
        // that out — see `placed_by_order`. Not a `text_order` per edit: each of
        // those opens the page.
        let text_objects = self.text_objects_in_order(page_index).unwrap_or_default();
        let resolve_total = t_resolve.elapsed();
        // Every run on the page, read the first time a line needs counting to
        // be found (below) and kept for the rest of the batch.
        let mut run_list: Option<Vec<crate::document::TextRun>> = None;
        // The operators already taken by an edit of this batch.
        let mut claimed: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let codes_in = |p: &content::Placed| codes_drawn(&file, fonts.as_ref(), &operations, p);
        // What each font used so far can really draw — see `subset`.
        let mut inked: HashMap<Vec<u8>, Option<std::collections::BTreeSet<u32>>> = HashMap::new();

        for &(object, text, requested_face, new_size, requested_color) in edits {
            let run = resolved
                .get(&object)
                .cloned()
                .ok_or_else(|| PdfError::InvalidArgument("that is not a text run".into()))?;
            let previous = run.text.clone();

            let order = text_objects.binary_search(&object).ok().map(|which| (which, text_objects.len()));
            let found = self.operator_of(page_index, &run, height, &placed, order, &codes_in, &mut run_list)?;
            // **One operator, one edit.** Two edits spliced into the same
            // operator keep the first and drop the second without a word
            // (`content::splice` skips what overlaps), so a batch that said both
            // were done would have done one. Two objects can only land on one
            // operator where one of them was found by position and found wrong:
            // refused, with nothing written, rather than half applied.
            if !claimed.insert(found.origin.operation) {
                return Err(PdfError::Unsupported("editing two runs that one operator draws"));
            }

            // **A colour-only change never touches the codes at all** —
            // wrap the operator in a fresh fill colour and restore the old
            // one after, the same shape `set_run_color_in_stream` already
            // uses for one run alone, needing none of the font-matching
            // below. The colour put back is the stream's own — see
            // `fill_before` — so a piece hidden in the middle of a line leaves
            // every piece after it drawn in exactly the colour it was.
            if let Some(color) = requested_color {
                let operation = &operations[found.origin.operation];
                let replacement = drawn_in_colour(&stream, &operations, found.origin.operation, color);
                spans.push((operation.span.clone(), replacement));
                results.push((
                    previous,
                    crate::document::TextStyle { color: Some(run.color), ..Default::default() },
                ));
                continue;
            }

            let name = found.font.clone().ok_or(PdfError::Unsupported("that text selects no font"))?;
            let fonts_ref = fonts.as_ref().ok_or(PdfError::Unsupported("that page declares no fonts"))?;
            let width = code_width(&file, fonts_ref, &name)
                .ok_or(PdfError::Unsupported("that font's codes cannot be counted"))?;
            // See `set_run_in_stream`: ASCII is what the codes mean, `ink` is
            // what the subset can actually draw. Asked once per font for the
            // whole batch — it reads the font program, and a paragraph is many
            // lines in the same few fonts.
            let mut ink = None;
            let unicode = match Self::font_to_unicode(&file, bytes, fonts_ref, &name) {
                Some(map) => map,
                None if width == 1 => {
                    ink = inked
                        .entry(name.clone())
                        .or_insert_with(|| crate::pdf::subset::drawable_ascii(&file, fonts_ref, &name))
                        .clone();
                    assumed_ascii()
                }
                None => {
                    return Err(PdfError::Unsupported("that font carries no character map this can read"))
                }
            };
            let reverse = spelling_codes(&unicode, ink.as_ref(), &run.text);
            let wanted = encodable(text, &reverse);

            let (encoded, swap) = if let Some(face) = requested_face {
                let (swap, encoded) =
                    self.embed_typing_font(&file, page_index, &page, &wanted, &reverse, Some(face))?;
                (encoded, Some(swap))
            } else {
                match encode_with(&reverse, &wanted, width) {
                    Some(bytes) => (bytes, None),
                    None => match self.borrow_font_on_page(&file, bytes, fonts_ref, &name, &wanted) {
                        Some((swap, encoded)) => (encoded, Some(swap)),
                        None => {
                            let (swap, encoded) =
                                self.embed_typing_font(&file, page_index, &page, &wanted, &reverse, None)?;
                            (encoded, Some(swap))
                        }
                    },
                }
            };

            let operation = &operations[found.origin.operation];
            let codes = codes_of(operation, width);
            let spellings: Vec<Option<String>> =
                codes.iter().map(|code| unicode.get(code).cloned()).collect();
            let owner = align_codes(&spellings, &run.text)
                .or_else(|| (codes.len() == run.text.chars().count()).then(|| (0..codes.len()).collect()))
                .ok_or(PdfError::Unsupported("that run's codes cannot be lined up with its text"))?;
            let span = owned_codes(&owner)
                .ok_or(PdfError::Unsupported("that run's characters belong to none of its codes"))?;
            let (first, last) = (span.start, span.end - 1);

            let drawing = swap.as_ref().map(|s| s.resource.clone()).unwrap_or_else(|| name.clone());
            let vertical_scale = if found.size.abs() > 1e-6 { run.size / found.size } else { 1.0 };
            let size = new_size
                .filter(|s| *s > 0.0)
                .map(|wanted| if vertical_scale.abs() > 1e-6 { wanted / vertical_scale } else { wanted })
                .unwrap_or(found.size);
            let mut replacement =
                content::replacing_codes(operation, first..last + 1, &encoded, &drawing, size, width);
            // The page's own font and size back after it — see `set_run_in_stream`.
            let mut after: Vec<u8> = Vec::new();
            if swap.is_some() || size != found.size {
                after = format!("\n/{} {} Tf", String::from_utf8_lossy(&name), found.size).into_bytes();
                replacement.extend_from_slice(&after);
            }
            // A `'` or `"` also moved to the next line before it drew, which the
            // `TJ` written in its place does not do: so that is said first, and
            // the lines after it keep their place.
            let moved = quote_effect(operation)?;
            if !moved.is_empty() {
                replacement = [moved.as_slice(), b"\n", replacement.as_slice()].concat();
            }
            spans.push((operation.span.clone(), replacement));

            // **A line to be spread to a width** is written plain here, like any
            // other, and written again below once its plain width is known. What
            // cannot be spread is refused now, before anything has been written:
            // text drawn at an angle or vertically (its box is not its width), and
            // a size that moves nothing.
            if let Some(&target) = stretch.get(&object) {
                if (found.axis.0 - 1.0).abs() > 1e-3 || found.axis.1.abs() > 1e-3 {
                    return Err(PdfError::InvalidArgument(
                        "this line is drawn at an angle, so it cannot be stretched to a width".into(),
                    ));
                }
                if font_is_vertical(&file, fonts_ref, &name) {
                    return Err(PdfError::InvalidArgument(
                        "this line is written vertically, so it cannot be stretched to a width".into(),
                    ));
                }
                let th = states.get(found.origin.operation).map_or(1.0, |s| s.horizontal_scale);
                let points_per_unit = size * found.scale * th / 1000.0;
                if !(points_per_unit.is_finite() && points_per_unit > 1e-9) {
                    return Err(PdfError::InvalidArgument(
                        "this line is drawn at a size that cannot be spaced, so it cannot be stretched to a width".into(),
                    ));
                }
                stretches.push(Stretch {
                    span: spans.len() - 1,
                    object,
                    target,
                    operation: found.origin.operation,
                    range: first..last + 1,
                    encoded: encoded.clone(),
                    text: wanted.clone(),
                    drawing: drawing.clone(),
                    size,
                    width,
                    after,
                    before: moved,
                    points_per_unit,
                });
            }

            if let Some(swap) = swap {
                if !swap.added.is_empty() || swap.page.is_some() {
                    embeds += 1;
                    if embeds > 1 {
                        return Err(PdfError::Unsupported(TOO_MANY_EMBEDS_IN_ONE_BATCH));
                    }
                    extra.extend(swap.added.iter().cloned());
                    page_patch = swap.page.clone();
                }
                face_written = Some(swap.face);
            }
            results.push((
                previous,
                crate::document::TextStyle { size: Some(run.size), ..Default::default() },
            ));
        }

        // ---- the pieces that come off the page
        //
        // Found the way an edit's run is, claimed so that no operator is both
        // edited and cut, and checked *all together* before any of them is
        // spliced: whether the pieces left behind keep their places depends on
        // which pieces are going, not on any one of them.
        let t_removals = std::time::Instant::now();
        let mut taking: Vec<(usize, usize)> = Vec::with_capacity(removals.len());
        for &object in removals {
            let run = resolved
                .get(&object)
                .ok_or_else(|| PdfError::InvalidArgument("that is not a text run".into()))?;
            let order = text_objects.binary_search(&object).ok().map(|which| (which, text_objects.len()));
            let found = self.operator_of(page_index, run, height, &placed, order, &codes_in, &mut run_list)?;
            if !claimed.insert(found.origin.operation) {
                return Err(PdfError::Unsupported("editing two runs that one operator draws"));
            }
            taking.push((object, found.origin.operation));
        }
        let going: std::collections::HashSet<usize> = taking.iter().map(|(_, at)| *at).collect();
        let mut carried: Vec<(usize, Vec<usize>)> = Vec::new();
        for &(object, at) in &taking {
            // Text that adds to the clipping path takes the clip with it.
            if render_mode_before(&operations, at) >= 4 {
                return Err(PdfError::Unsupported("taking out text that is drawn as a clipping path"));
            }
            let after = carried_after_removal(&operations, at, &going);
            if !after.is_empty() {
                carried.push((object, after));
            }
            // The operator and its operands, whole — and, for a quote operator,
            // the line it moves to, which the lines after it are placed from.
            spans.push((operations[at].span.clone(), quote_effect(&operations[at])?));
        }
        // **A piece placed by the pen cannot lose the piece before it** — if it
        // draws anything. The pen carries on from a piece that goes into whatever
        // follows it with nothing repositioning it; a piece that stays and is seen
        // would move. One that is not seen can be left to move: a line that
        // Illustrator wrote with a space of its own between its words is the
        // ordinary case, and those spaces draw nothing.
        if !carried.is_empty() {
            let judged: Vec<usize> = carried.iter().flat_map(|(_, after)| after.iter().copied()).collect();
            let seen = self.draws_something(page_index, &operations, &placed, &text_objects, &resolved, height, &codes_in, &judged)?;
            for (object, after) in &carried {
                if after.iter().any(|op| seen.get(op).copied().unwrap_or(true)) {
                    return Err(PdfError::InvalidArgument(format!(
                        "object {object} cannot come off the page: a piece after it on its line that \
                         draws something has nothing repositioning it, so it would move"
                    )));
                }
            }
        }
        timing.push(("  of which taking pieces off", t_removals.elapsed()));
        timing.push(("per_edit_loop_total", t_loop.elapsed()));
        timing.push(("  of which reading the runs and their order", resolve_total));

        // The document as these spans make it: the content stream spliced, packed
        // and written out, with the font objects the batch added. Written twice
        // when a line is to span a width (below), so it is a closure.
        let write = |spans: &[(std::ops::Range<usize>, Vec<u8>)]| -> Result<(Vec<u8>, std::time::Duration, std::time::Duration)> {
            let t = std::time::Instant::now();
            let edited = content::splice(&stream, spans);
            let mut replacements = Vec::new();
            for (index, (number, dict)) in streams.iter().enumerate() {
                let data = if index == 0 { edited.clone() } else { Vec::new() };
                let packed = content::encode(&data)?;
                let mut dict = dict.clone();
                dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
                dict.remove(b"DecodeParms");
                replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
            }
            if let Some(patch) = &page_patch {
                replacements.push(patch.clone());
            }
            let packed_in = t.elapsed();
            let t = std::time::Instant::now();
            let rewritten = Self::write_edit(&base, &file, &replacements, &extra)?;
            Ok((rewritten, packed_in, t.elapsed()))
        };

        // ---- the lines that are to span a width
        //
        // **Written plain first, to a copy that is only read.** The width a line
        // comes to depends on the font's glyph advances, on any `Tc` and `Tw`
        // and `Tz` in force where it is drawn, and on where PDFium puts the right
        // edge of the last glyph — and PDFium already knows all of it. So the whole
        // batch is written as it is without the stretching, opened as a document
        // of its own, and the retyped runs' boxes read from it; that copy is
        // dropped, and `self` is given only the version with the gaps opened.
        // Nothing here has touched `self`: a refusal leaves the page as it was.
        if !stretches.is_empty() {
            let t_stretch = std::time::Instant::now();
            let (plain, _, _) = write(&spans)?;
            let copy = Self::open_bytes(plain, None)?;
            // The page's objects are numbered as the copy now has them: a piece
            // taken off lowers the number of every object after it, and an edit
            // replaces one operator with one.
            let renumbered = |object: usize| object - removals.iter().filter(|r| **r < object).count();
            let wanted_now: std::collections::HashSet<usize> = stretches.iter().map(|s| renumbered(s.object)).collect();
            let measured: HashMap<usize, crate::document::TextRun> = copy
                .text_runs_some(page_index, &wanted_now)?
                .into_iter()
                .map(|run| (run.object, run))
                .collect();
            for stretching in &stretches {
                let run = measured.get(&renumbered(stretching.object)).ok_or_else(|| {
                    PdfError::InvalidArgument("this line could not be measured, so it cannot be stretched to a width".into())
                })?;
                // The retyped run is where the original was: its origin is the
                // operator's own, and the retype does not move it. If it is not,
                // the numbers did not line up and what was measured is something else.
                let was = &resolved[&stretching.object];
                if (run.origin.x - was.origin.x).abs() > 0.05 || (run.origin.y - was.origin.y).abs() > 0.05 {
                    return Err(PdfError::InvalidArgument(
                        "this line could not be measured, so it cannot be stretched to a width".into(),
                    ));
                }
                let plain_width = run.rect.right - run.rect.left;
                if !(plain_width.is_finite() && plain_width > 0.0) {
                    return Err(PdfError::InvalidArgument(
                        "this line has no width to measure, so it cannot be stretched to a width".into(),
                    ));
                }
                spans[stretching.span].1 = stretching.spread(&operations[stretching.operation], plain_width)?;
            }
            timing.push(("  of which measuring the lines to stretch", t_stretch.elapsed()));
        }

        let (rewritten, packed_in, written_in) = write(&spans)?;
        timing.push(("splice_and_pack", packed_in));
        timing.push(("write_edit", written_in));

        let t = std::time::Instant::now();
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)?;
        timing.push(("adopt_edit", t.elapsed()));

        self.substituted = face_written;
        self.last_batch_timing = timing;
        Ok(results)
    }

    /// [`DocumentMut::replace_text_lines`] for a batch in which more than one line
    /// needs a new font embedded: each first piece is written on its own, as
    /// [`DocumentMut::set_text_run_styled`] writes it, and then every piece to
    /// remove comes off in one pass.
    ///
    /// **Atomic all the same**, by a copy of the page taken first and put back on
    /// any failure — which is what the one-line-at-a-time fallback of
    /// `set_text_runs_styled` does not have, and could not have here, where the
    /// removal would be left undone. No object is renumbered by the writes (each
    /// replaces one show-text operator with one), so the objects to remove are
    /// still the ones that were asked for when their turn comes.
    fn replace_text_lines_one_at_a_time(
        &mut self,
        page_index: usize,
        requests: &[(usize, &str, Option<&str>, Option<f32>, Option<crate::document::Color>)],
        removals: &[usize],
        stretch: &HashMap<usize, f32>,
    ) -> Result<()> {
        let page = self.snapshot_page(page_index)?;
        let outcome = (|| -> Result<()> {
            for &(first, text, face, size, _) in requests {
                // A line that is to be stretched is written the way the batch
                // writes it, as a batch of one: that is where the stretching is.
                if let Some(width) = stretch.get(&first) {
                    self.set_runs_in_stream(
                        page_index,
                        &[(first, text, face, size, None)],
                        &[],
                        &HashMap::from([(first, *width)]),
                    )?;
                    continue;
                }
                let style = crate::document::TextStyle { face: face.map(str::to_string), size, ..Default::default() };
                self.set_text_run_styled(page_index, first, text, &style)?;
            }
            if !removals.is_empty() {
                self.set_runs_in_stream(page_index, &[], removals, &HashMap::new())?;
            }
            Ok(())
        })();
        if let Err(first) = &outcome {
            if let Err(restore) = self.delete_page(page_index).and_then(|_| self.insert_page(page_index, page)) {
                return Err(PdfError::Pdfium(format!("{first}; and the page could not be put back: {restore}")));
            }
        }
        outcome
    }

    /// For each of these show-text operations, whether the object that drew it
    /// draws something (`true`) or nothing ([`draws_nothing`]).
    ///
    /// **`true` wherever it cannot be told**: moving something that is seen is the
    /// one thing taking a piece out must not do unawares, so an operation that
    /// does not lead to its object through the verified count — [`placed_by_order`],
    /// held to the stream like every other use of it — is judged to draw
    /// something. `known` are runs already read; the others are read together, in
    /// one page open.
    #[allow(clippy::too_many_arguments)]
    fn draws_something(
        &self,
        page_index: usize,
        operations: &[crate::pdf::content::Operation],
        placed: &[crate::pdf::content::Placed],
        text_objects: &[usize],
        known: &HashMap<usize, crate::document::TextRun>,
        height: f32,
        codes_in: &dyn Fn(&crate::pdf::content::Placed) -> usize,
        wanted: &[usize],
    ) -> Result<HashMap<usize, bool>> {
        let mut out: HashMap<usize, bool> = wanted.iter().map(|op| (*op, true)).collect();
        // The count is the only way from an operation to its object, and only
        // where it holds for the page.
        if text_objects.len() != placed.len() {
            return Ok(out);
        }
        let ordinal_of: HashMap<usize, usize> =
            placed.iter().enumerate().map(|(k, p)| (p.origin.operation, k)).collect();
        // (operation, its place among the show-text operators, its object)
        let mut judged: Vec<(usize, usize, usize)> = Vec::new();
        for &op in wanted {
            if let Some(&k) = ordinal_of.get(&op) {
                judged.push((op, k, text_objects[k]));
            }
        }
        let missing: std::collections::HashSet<usize> =
            judged.iter().map(|j| j.2).filter(|o| !known.contains_key(o)).collect();
        let fetched: HashMap<usize, crate::document::TextRun> = if missing.is_empty() {
            HashMap::new()
        } else {
            self.text_runs_some(page_index, &missing)?.into_iter().map(|r| (r.object, r)).collect()
        };
        for (op, k, object) in judged {
            let Some(run) = known.get(&object).or_else(|| fetched.get(&object)) else { continue };
            let agreed = placed_by_order(run, height, placed, Some((k, text_objects.len())), codes_in)
                .is_some_and(|p| p.origin.operation == op);
            if agreed {
                out.insert(op, !draws_nothing(run, render_mode_before(operations, op)));
            }
        }
        Ok(out)
    }

    /// The show-text operator that drew `run`: **by the verified count** where
    /// the stream bears it out ([`placed_by_order`]), then by position, then by
    /// counting against every run of the page. `run_list` is that every-run list,
    /// read the first time it is needed and kept by the caller for the rest of a
    /// batch.
    fn operator_of<'a>(
        &self,
        page_index: usize,
        run: &crate::document::TextRun,
        height: f32,
        placed: &'a [crate::pdf::content::Placed],
        order: Option<(usize, usize)>,
        codes_in: &dyn Fn(&crate::pdf::content::Placed) -> usize,
        run_list: &mut Option<Vec<crate::document::TextRun>>,
    ) -> Result<&'a crate::pdf::content::Placed> {
        let (want_x, want_y) = (run.origin.x, height - run.origin.y);
        if let Some(found) = placed_by_order(run, height, placed, order, codes_in)
            .or_else(|| nearest_placed(placed, want_x, want_y).map(|(at, _)| &placed[at]))
        {
            return Ok(found);
        }
        // **Where it is drawn is not always where the walk thinks.** The text
        // matrix advances by the width of whatever was just shown, and working
        // that out needs the font's glyph widths, which the walk does not have,
        // so a piece that continues another is reported at the first one's
        // origin. Counting is the way out — but only where the page has *shown*
        // that counting works. See `placed_in_order`.
        if run_list.is_none() {
            *run_list = Some(self.text_runs_all(page_index)?);
        }
        let run_list = run_list.as_deref().unwrap_or_default();
        let index = run_list
            .iter()
            .position(|r| r.object == run.object)
            .ok_or(PdfError::Unsupported("that is not a text run"))?;
        placed_in_order(run_list, placed, height, index)
            .ok_or(PdfError::Unsupported("that text is drawn in a way this cannot edit"))
    }

    /// Change a run's fill colour without letting PDFium anywhere near the
    /// page's own content.
    ///
    /// Colour used to fall straight through to `set_text_run_styled`'s slow
    /// path, since it is neither a text change nor `set_run_in_stream`'s own
    /// font/size territory. That path's `FPDFPage_GenerateContent` rebuilds
    /// the whole stream from PDFium's object model — the same regeneration
    /// that reorders an untouched paragraph on at least one real page — and
    /// its scramble guard then refuses the edit outright rather than risk
    /// it, which meant a run's colour could not be changed on that page at
    /// all.
    ///
    /// A colour is not a new glyph: the codes drawing this run do not
    /// change, so nothing here needs `set_run_in_stream`'s font-matching or
    /// text encoding — only the same operator lookup, and the same
    /// restore-after trick it already uses for a swapped `Tf`.
    fn set_run_color_in_stream(
        &mut self,
        page_index: usize,
        object: usize,
        color: crate::document::Color,
    ) -> Result<crate::document::Color> {
        use crate::pdf::content;

        let run_list = self.text_runs_all(page_index)?;
        let run = run_list
            .iter()
            .find(|r| r.object == object)
            .cloned()
            .ok_or_else(|| PdfError::InvalidArgument("that is not a text run".into()))?;
        let previous = run.color;
        let height = self.page_size(page_index)?.height_pt;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;

        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        // The same lookup `set_run_in_stream` uses — by count where the stream
        // bears it out, then by position, then by counting against every run —
        // see its own doc for why each is needed.
        let (want_x, want_y) = (run.origin.x, height - run.origin.y);
        let fonts = self.page_fonts(&file, &page);
        let order = self.text_order(page_index, object);
        let codes_in = |p: &content::Placed| codes_drawn(&file, fonts.as_ref(), &operations, p);
        let found = match placed_by_order(&run, height, &placed, order, &codes_in)
            .or_else(|| nearest_placed(&placed, want_x, want_y).map(|(at, _)| &placed[at]))
        {
            Some(found) => found,
            None => {
                let index = run_list
                    .iter()
                    .position(|r| r.object == object)
                    .ok_or(PdfError::Unsupported("that is not a text run"))?;
                placed_in_order(&run_list, &placed, height, index).ok_or(PdfError::Unsupported(
                    "that text is drawn in a way this cannot edit",
                ))?
            }
        };

        // Put the page's own colour back. Colour is graphics state exactly
        // like the font `set_run_in_stream` restores after a swap: it stays
        // selected until something changes it again, so anything drawn
        // after this in the same text object would otherwise come out in
        // this run's new colour too. The stream's own colour operators, not a
        // stand-in for what PDFium read — see `fill_before`.
        let replacement = drawn_in_colour(&stream, &operations, found.origin.operation, color);
        let operation = &operations[found.origin.operation];

        let edited = content::splice(&stream, &[(operation.span.clone(), replacement)]);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &[])?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)?;
        Ok(previous)
    }

    /// Take the marks that cover an area off the page, and say how many.
    ///
    /// Removed back to front, because removing one renumbers everything after
    /// it — walking forwards deletes the wrong marks as soon as there are two.
    fn hide_annotations_in(&mut self, page_index: usize, area: &Rect) -> Result<usize> {
        let over: Vec<usize> = self
            .annotations(page_index)?
            .into_iter()
            .filter(|found| {
                found.annotation.bounds().is_some_and(|b| {
                    b.left < area.right
                        && b.right > area.left
                        && b.top < area.bottom
                        && b.bottom > area.top
                })
            })
            .map(|found| found.index)
            .collect();

        let mut removed = 0usize;
        for index in over.into_iter().rev() {
            // One that will not come off is reported by the caller's own count
            // rather than failing the lock — the words still went, and the
            // sealed copy still has everything.
            if self.remove_annotation(page_index, index).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Bytes from the operating system's own source of randomness.
    ///
    /// Panics rather than falling back to anything weaker: a password whose
    /// salts are predictable is not a password, and carrying on quietly would
    /// be the worst possible answer.
    fn randomness(buffer: &mut [u8]) {
        use rand_core::RngCore;
        rand_core::OsRng.fill_bytes(buffer);
    }

    /// The document saved with its existing password taken off.
    ///
    /// **Checked, not trusted.** The flag's value differs between PDFium
    /// releases — 3 in older, 4 in newer — and a wrong one would write an
    /// encrypted file while this code believed it plain, which is the worst
    /// outcome available. So the result is read back, and a file that still
    /// carries an `/Encrypt` dictionary is refused rather than handed over as a
    /// plain copy.
    /// The document's own bytes, as **this crate's reader** can read them.
    ///
    /// **PDFium keeps a document's encryption when it saves one it opened
    /// encrypted** — which is right for saving and wrong for reading. Every
    /// content-stream edit in this file works on the bytes PDFium would write,
    /// and an encrypted stream is ciphertext to a reader holding no key: it
    /// inflates to nothing and comes back as "a content stream filter this
    /// build cannot read".
    ///
    /// Reported from use, on a catalogue with a password: the same run edited
    /// cleanly before the password went on and was refused after. So a secured
    /// document is written out *without* its security for the read — and the
    /// password is put back by [`Self::rearm_security`] once the rewrite lands,
    /// because a document that quietly lost its password would be far worse
    /// than one that could not be edited.
    fn readable_bytes(&self) -> Result<Vec<u8>> {
        if self.already_secured {
            return self.without_password();
        }
        self.document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))
    }

    /// Put back the password a rewrite dropped.
    ///
    /// After [`Self::readable_bytes`] the document in hand is plaintext, and
    /// reopening it leaves nothing to re-encrypt on the next save. This arms the
    /// same pending-password state `secure_document` would, using the password
    /// the document was opened with.
    ///
    /// **The owner password cannot come back**: only the one that opened the
    /// file is known. A document that had a separate owner password keeps its
    /// user password and loses that distinction, which is said here rather than
    /// discovered.
    fn rearm_security(&mut self, was_secured: bool, plus: bool, permissions: Option<crate::pdf::encrypt::Permissions>) {
        // A password on its way off stays off. Re-arming it here put it back
        // on any document that was edited after `unsecure` — the edit read
        // the plaintext, and the read's own bookkeeping restored what the
        // person had just removed. Found by audit.
        if !was_secured || self.remove_password {
            return;
        }
        let Some(user) = self.opened_with.clone() else { return };
        self.secure_plus = plus;
        self.security = Some(Wanted {
            user,
            owner: None,
            permissions: permissions.unwrap_or_else(crate::pdf::encrypt::Permissions::all),
        });
    }

    fn without_password(&self) -> Result<Vec<u8>> {
        const REMOVE_SECURITY: u32 = 3;
        let bytes = self.save_with_flags(REMOVE_SECURITY)?;

        let file = crate::pdf::File::parse(&bytes)?;
        if file.trailer().get(b"Encrypt").is_some() {
            return Err(PdfError::Unsupported(
                "this build of PDFium would not take the password off",
            ));
        }
        Ok(bytes)
    }

    /// A secured copy of the bytes PDFium saved.
    fn encrypted(&self, bytes: &[u8], wanted: &Wanted) -> Result<Vec<u8>> {
        let file = crate::pdf::File::parse(bytes)?;
        if self.secure_plus {
            let plus = crate::pdf::secure_plus::SecurePlus::new(
                &wanted.user,
                crate::crypto::kdf::KdfParams::default(),
                Self::randomness,
            )?;
            return crate::pdf::secure_plus::secure(&file, &plus);
        }
        let security = crate::pdf::encrypt::Security::new(
            &wanted.user,
            wanted.owner.as_deref().map(|o| o.as_slice()),
            wanted.permissions,
            Self::randomness,
        )?;
        crate::pdf::encrypt::secure(&file, &security, Self::randomness)
    }

    /// Append operators to a page's own content.
    ///
    /// Through `crate::pdf` rather than PDFium, for the reason that module
    /// exists: `FPDFPage_GenerateContent` would re-emit the page and rewrite
    /// text nobody asked it to.
    ///
    /// **A page with no content stream at all gets one.** They exist — a blank
    /// sheet in a form, a separator page — and refusing to put a tick on one
    /// because there is nothing to append to would be a strange answer to a
    /// reasonable request.
    fn append_to_page(&mut self, page_index: usize, operators: &[u8]) -> Result<()> {
        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let bytes = &base.bytes;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;

        let mut replacements = Vec::new();
        let mut extra = Vec::new();
        match self.page_content(&file, &page) {
            Ok((mut stream, streams)) => {
                stream.extend_from_slice(operators);
                for (index, (number, dict)) in streams.iter().enumerate() {
                    // Everything into the first stream, the rest emptied — the
                    // only shape that keeps a multi-part content stream
                    // consistent.
                    let data = if index == 0 { stream.clone() } else { Vec::new() };
                    let packed = crate::pdf::content::encode(&data)?;
                    let mut dict = dict.clone();
                    dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
                    dict.remove(b"DecodeParms");
                    replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
                }
            }
            Err(_) => {
                // No content stream: make one, and point the page at it.
                let number = file.next_object_number()?;
                let packed = crate::pdf::content::encode(operators)?;
                let mut dict = crate::pdf::Dict(Vec::new());
                dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
                extra.push((number, crate::pdf::write_stream(&dict, &packed)));

                let page_number = self.page_object_number(&file, page_index)?;
                let Ok(crate::pdf::Object::Dict(mut dict)) = file.object(page_number) else {
                    return Err(PdfError::InvalidArgument("that page cannot be written".into()));
                };
                dict.set(b"Contents", crate::pdf::Object::Reference(number, 0));
                let mut body = Vec::new();
                crate::pdf::write_object(&mut body, &crate::pdf::Object::Dict(dict));
                replacements.push((page_number, body));
            }
        }

        let rewritten = Self::write_edit(&base, &file, &replacements, &extra)?;
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// A page's content, decoded, and which stream objects it came from.
    fn page_content(
        &self,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
    ) -> Result<(Vec<u8>, Vec<(u32, crate::pdf::Dict)>)> {
        use crate::pdf::{content, Object};

        let contents = page
            .as_dict()
            .and_then(|d| d.get(b"Contents"))
            .ok_or_else(|| PdfError::InvalidArgument("that page draws nothing".into()))?;

        // One stream, or several to be read end to end. Both are ordinary, and
        // so is a **reference to** the array of them — `/Contents 9 0 R` where
        // object 9 is `[7 0 R 8 0 R 10 0 R]`. Not following that reference made
        // this refuse on documents whose pages are perfectly ordinary, and with
        // it the whole content-stream path went unused on them: locking fell
        // back to PDFium and editing rewrote the page.
        let parts: Vec<Object> = match contents {
            Object::Array(items) => items.clone(),
            reference @ Object::Reference(..) => match file.resolve(reference)? {
                Object::Array(items) => items,
                // A reference to a stream is the common case; keep the
                // reference itself, because the object number is needed below.
                _ => vec![reference.clone()],
            },
            other => vec![other.clone()],
        };

        let mut stream = Vec::new();
        let mut streams = Vec::new();
        for part in parts {
            let Some((number, _)) = part.as_reference() else {
                return Err(PdfError::Unsupported("a content stream written inline"));
            };
            let Object::Stream(dict, range) = file.object(number)? else {
                return Err(PdfError::InvalidArgument("the contents are not a stream".into()));
            };
            let decoded = content::decode(&dict, &file.bytes()[range]).ok_or(
                PdfError::Unsupported("a content stream filter this build cannot read"),
            )?;
            stream.extend_from_slice(&decoded);
            // A newline between them: the parts may split mid-operator only by
            // accident, and joining them without one would fuse two tokens.
            stream.push(b'\n');
            streams.push((number, dict));
        }
        if streams.is_empty() {
            return Err(PdfError::InvalidArgument("that page draws nothing".into()));
        }
        Ok((stream, streams))
    }

    /// Make every lock real in the document itself, now.
    ///
    /// **Not at save time.** A lock that only becomes real when somebody
    /// remembers to save is not a lock — it is a promise, and the gap between
    /// the two is where a document gets sent to a partner with the picture
    /// still in it. So the file is rewritten here, and what is in memory
    /// afterwards genuinely no longer holds the image.
    ///
    /// The cost is a save and a reopen per lock, seconds on a large catalogue.
    /// That is the price of the edit not going through
    /// `FPDFPage_GenerateContent`, which would re-emit every content stream on
    /// the page and rewrite text nobody touched — see [`crate::pdf`].
    fn apply_locks(&mut self) -> Result<()> {
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let rewritten = self.blank_locked_images(bytes)?;

        // No password: what PDFium just wrote is not encrypted, whatever the
        // document was opened from.
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        // A different document object now holds the attachment, so anything
        // remembered about the old one is stale.
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        // The source stays as it was — this is still the same document, from
        // wherever it came from — and it is still unsaved.
        self.touch();
        // A document carrying a lock must be saved as a full copy, or the
        // image stays in the file's earlier revision.
        self.redacted = true;
        Ok(())
    }

    /// Empty every locked image in a file's bytes.
    ///
    /// A file this cannot account for is refused rather than half-rewritten: an
    /// image still in the file while the vault says it is hidden is the one
    /// outcome worse than not locking it at all.
    fn blank_locked_images(&self, bytes: Vec<u8>) -> Result<Vec<u8>> {
        let Some(vault) = self.read_vault()? else { return Ok(bytes) };
        let wanted: Vec<crate::crypto::vault::SealedItem> = vault
            .items
            .iter()
            .filter(|i| i.object != usize::MAX)
            .cloned()
            .collect();
        if wanted.is_empty() {
            return Ok(bytes);
        }

        let file = crate::pdf::File::parse(&bytes)?;
        let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
        for item in &wanted {
            let number = self.image_object_number(&file, item)?;
            if replacements.iter().any(|(n, _)| *n == number) {
                continue;
            }
            replacements.push((number, blank_image_object()));
        }
        file.rewrite(&replacements)
    }

    /// The object number of the *n*th picture a page draws, found through the
    /// `Do` that draws it.
    ///
    /// **The exact answer**, where matching on pixel size is a guess that fails
    /// on the ordinary case of a page carrying two images the same size —
    /// reported from use as *"two images of the same size on one page, which
    /// cannot be told apart"* on a brochure, where it left a padlock over an
    /// image that was still perfectly visible.
    fn image_drawn_as(
        &self,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
        which: usize,
        pictures: usize,
    ) -> Result<u32> {
        use crate::pdf::{content, Object};

        let names = self.image_names(file, page)?;
        let (stream, _) = self.page_content(file, page)?;
        let operations = content::parse(&stream)?;
        let drawn = image_operators(&operations, &names);

        // The evidence, as everywhere else this walks a stream: as many
        // pictures drawn as PDFium reports. Without it the *n*th `Do` is not
        // necessarily the *n*th picture.
        if drawn.len() != pictures {
            return Err(PdfError::Unsupported(
                "this page draws its pictures in a way this cannot follow",
            ));
        }
        let at = *drawn
            .get(which)
            .ok_or(PdfError::Unsupported("that picture is not drawn on this page"))?;
        let Some(Object::Name(name)) = operations[at].operands.first() else {
            return Err(PdfError::Unsupported("that picture is drawn without a name"));
        };

        let resources = self
            .inherited(file, page, b"Resources")?
            .ok_or_else(|| PdfError::InvalidArgument("that page declares no resources".into()))?;
        let xobjects = resources
            .as_dict()
            .and_then(|d| d.get(b"XObject"))
            .map(|found| file.resolve(found))
            .transpose()?
            .ok_or_else(|| PdfError::InvalidArgument("that page draws no images".into()))?;

        xobjects
            .as_dict()
            .and_then(|d| d.get(name).cloned())
            .and_then(|entry| entry.as_reference().map(|(number, _)| number))
            .ok_or_else(|| {
                PdfError::InvalidArgument("that picture is not among the page's resources".into())
            })
    }

    /// Which PDF object holds one locked image.
    ///
    /// Reached through the page's own resources rather than by searching the
    /// whole file: two pages may draw the same image, and blanking it because
    /// one of them locked it would empty it on the other as well.
    fn image_object_number(
        &self,
        file: &crate::pdf::File<'_>,
        item: &crate::crypto::vault::SealedItem,
    ) -> Result<u32> {
        use crate::pdf::Object;

        let page = self.page_object(file, item.page_index)?;
        let resources = self.inherited(file, &page, b"Resources")?.ok_or_else(|| {
            PdfError::InvalidArgument("that page declares no resources".into())
        })?;
        let xobjects = match resources.as_dict().and_then(|d| d.get(b"XObject")) {
            Some(found) => file.resolve(found)?,
            None => {
                return Err(PdfError::InvalidArgument("that page draws no images".into()))
            }
        };

        // **Which name the page draws it under, read from the page itself.**
        //
        // The record keeps PDFium's object index, which counts every object on
        // the page; the resource dictionary is keyed by name and unordered. The
        // content stream is what joins them: the Nth image PDFium reports is
        // drawn by the Nth image-drawing `Do`, and that operator carries the
        // name. Exact, where anything inferred from the image itself is a
        // guess.
        let pictures = self.images_on(item.page_index)?;
        if let Some(which) = pictures.iter().position(|i| i.object == item.object) {
            if let Ok(number) = self.image_drawn_as(file, &page, which, pictures.len()) {
                return Ok(number);
            }
        }

        // Failing that — a page whose pictures this cannot follow in the stream
        // — the two are matched on what the image *is*, by the pixel size
        // PDFium reported when it was locked.
        let locked = pictures
            .into_iter()
            .find(|i| i.object == item.object)
            .ok_or_else(|| PdfError::InvalidArgument("the locked image has moved".into()))?;

        let mut found = None;
        for (_, value) in xobjects.as_dict().map(|d| d.0.clone()).unwrap_or_default() {
            let Some((number, _)) = value.as_reference() else { continue };
            let Ok(Object::Stream(dict, _)) = file.object(number) else { continue };
            if dict.get(b"Subtype").and_then(Object::as_name) != Some(&b"Image"[..]) {
                continue;
            }
            let width = dict.get(b"Width").and_then(Object::as_i64).unwrap_or(-1);
            let height = dict.get(b"Height").and_then(Object::as_i64).unwrap_or(-1);
            if width == i64::from(locked.pixel_width) && height == i64::from(locked.pixel_height)
            {
                // Two images of identical pixel size on one page cannot be told
                // apart this way, and blanking the wrong one cannot be undone —
                // so it is refused rather than guessed at.
                if found.is_some() {
                    return Err(PdfError::Unsupported(
                        "two images of the same size on one page, which cannot be told apart",
                    ));
                }
                found = Some(number);
            }
        }
        found.ok_or_else(|| {
            PdfError::InvalidArgument("the locked image is not among the page's resources".into())
        })
    }

    /// A key from a page, or from the nearest parent that declares one.
    ///
    /// `/Resources` is inheritable — a page tree may declare it once at the top
    /// and never again — so reading it from the page alone finds nothing on
    /// perfectly ordinary files.
    fn inherited(
        &self,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
        key: &[u8],
    ) -> Result<Option<crate::pdf::Object>> {
        let mut node = page.clone();
        for _ in 0..64 {
            let Some(dict) = node.as_dict() else { return Ok(None) };
            if let Some(found) = dict.get(key) {
                return file.resolve(found).map(Some);
            }
            let Some(parent) = dict.get(b"Parent") else { return Ok(None) };
            node = file.resolve(parent)?;
        }
        Err(PdfError::InvalidArgument("the page tree nests too deeply".into()))
    }

    /// One page's dictionary, walked down from the catalogue.
    fn page_object(
        &self,
        file: &crate::pdf::File<'_>,
        page_index: usize,
    ) -> Result<crate::pdf::Object> {
        let root = file.resolve(
            file.trailer()
                .get(b"Root")
                .ok_or_else(|| PdfError::InvalidArgument("the file has no catalogue".into()))?,
        )?;
        let pages = file.resolve(
            root.as_dict()
                .and_then(|d| d.get(b"Pages"))
                .ok_or_else(|| PdfError::InvalidArgument("the file has no page tree".into()))?,
        )?;

        let mut flat = Vec::new();
        collect_pages(file, &pages, &mut flat, &mut HashSet::new(), 0)?;
        flat.into_iter().nth(page_index).ok_or_else(|| {
            PdfError::InvalidArgument(format!("the file has no page {}", page_index + 1))
        })
    }

    /// Which object number a page is.
    ///
    /// [`Self::page_object`] returns the dictionary, which is enough to read a
    /// page but not to replace one — writing it back needs the number. Walked
    /// separately rather than returned alongside, because every other caller
    /// wants the dictionary and nothing else.
    fn page_object_number(
        &self,
        file: &crate::pdf::File<'_>,
        page_index: usize,
    ) -> Result<u32> {
        fn walk(
            file: &crate::pdf::File<'_>,
            node: &crate::pdf::Object,
            out: &mut Vec<u32>,
            seen: &mut HashSet<u32>,
            depth: usize,
        ) {
            if depth > 64 {
                return;
            }
            let Some(dict) = node.as_dict() else { return };
            let Some(kids) = dict.get(b"Kids") else { return };
            let Ok(crate::pdf::Object::Array(items)) = file.resolve(kids) else { return };
            for kid in items {
                let crate::pdf::Object::Reference(number, _) = kid else { continue };
                // A page tree that names the same child at every level is a
                // DAG, not a tree — the depth cap alone lets that double the
                // work at every level instead of stopping it. Found by audit.
                if !seen.insert(number) {
                    continue;
                }
                let Ok(resolved) = file.object(number) else { continue };
                let is_page = resolved
                    .as_dict()
                    .and_then(|d| d.get(b"Type"))
                    .and_then(crate::pdf::Object::as_name)
                    == Some(&b"Page"[..]);
                if is_page {
                    out.push(number);
                } else {
                    walk(file, &resolved, out, seen, depth + 1);
                }
            }
        }

        let root = file.resolve(
            file.trailer()
                .get(b"Root")
                .ok_or_else(|| PdfError::InvalidArgument("the file has no catalogue".into()))?,
        )?;
        let pages = file.resolve(
            root.as_dict()
                .and_then(|d| d.get(b"Pages"))
                .ok_or_else(|| PdfError::InvalidArgument("the file has no page tree".into()))?,
        )?;
        let mut numbers = Vec::new();
        walk(file, &pages, &mut numbers, &mut HashSet::new(), 0);
        numbers.into_iter().nth(page_index).ok_or_else(|| {
            PdfError::InvalidArgument(format!("the file has no page {}", page_index + 1))
        })
    }

    /// The lock this document carries, if it carries one.
    ///
    /// Matched by name and then by magic. A document may hold attachments for
    /// any number of reasons and one of them being unreadable as a vault is not
    /// a fault in the document — but a file named ours whose contents are not
    /// is, and that comes back as an error rather than as `None`.
    /// The lock, read from the file at most once.
    ///
    /// Every caller goes through here — see the `vault` field for what reading
    /// it afresh each time cost. Invalidated by `write_vault` and by anything
    /// that reopens the document, which between them are the only ways the
    /// attachment can change.
    fn read_vault(&self) -> Result<Option<Vault>> {
        let mut cached = self.vault.lock().map_err(|_| {
            PdfError::Pdfium("the lock cache was poisoned by an earlier panic".into())
        })?;
        if let Some(known) = cached.as_ref() {
            return match known {
                Ok(vault) => Ok(vault.clone()),
                Err(why) => Err(PdfError::InvalidArgument(why.clone())),
            };
        }
        let read = self.read_vault_uncached();
        *cached = Some(match &read {
            Ok(vault) => Ok(vault.clone()),
            Err(e) => Err(e.to_string()),
        });
        read
    }

    fn read_vault_uncached(&self) -> Result<Option<Vault>> {
        let bindings = pdfium()?.bindings();
        let handle = self.document.handle();
        let count = unsafe { bindings.FPDFDoc_GetAttachmentCount(handle) };

        for index in 0..count {
            let attachment = unsafe { bindings.FPDFDoc_GetAttachment(handle, index) };
            if attachment.is_null() || attachment_name(bindings, attachment) != vault::ATTACHMENT {
                continue;
            }

            let mut needed: c_ulong = 0;
            unsafe {
                bindings.FPDFAttachment_GetFile(attachment, std::ptr::null_mut(), 0, &mut needed)
            };
            if needed == 0 {
                return Err(PdfError::InvalidArgument(
                    "this document's lock is empty".into(),
                ));
            }
            let mut buffer = vec![0u8; needed as usize];
            let read = unsafe {
                bindings.FPDFAttachment_GetFile(
                    attachment,
                    buffer.as_mut_ptr() as *mut c_void,
                    needed,
                    &mut needed,
                )
            } != 0;
            if !read {
                return Err(PdfError::InvalidArgument(
                    "this document's lock could not be read".into(),
                ));
            }
            buffer.truncate(needed as usize);
            return Vault::parse(&buffer).map(Some);
        }
        Ok(None)
    }

    /// Put the vault into the document, replacing any earlier one.
    ///
    /// `FPDFDoc_AddAttachment` refuses a name that already exists, so the old
    /// entry goes first. Note what PDFium's own documentation says about that:
    /// deleting an attachment removes its entry from the name tree and **not**
    /// its data from the file. That is survivable here only because everything
    /// in an old vault was sealed under the same passcode as the new one and
    /// describes pages this document still has — and because a locked document
    /// must be saved as a full copy anyway, which is what actually compacts it.
    ///
    /// **`dek` re-binds `pages` and `items` before anything is written** — see
    /// `Vault::seal_structure`. `repair_locks` is the one caller with no
    /// passcode in hand at all (it only ever drops a lock nothing can find its
    /// way back to, never opens one); rather than write a binding that no
    /// longer matches and have the next real unlock call that tampering, the
    /// binding is cleared instead, and picked up again on this document's next
    /// passcode-bearing write. Found by audit.
    fn write_vault(&mut self, vault: &mut Vault, dek: Option<&Secret>) -> Result<()> {
        match dek {
            Some(dek) => vault.seal_structure(dek)?,
            None => vault.bind = None,
        }
        // What was cached is now what is being replaced.
        if let Ok(mut cached) = self.vault.lock() {
            *cached = Some(Ok(Some(vault.clone())));
        }
        let bytes = vault.to_bytes()?;
        let bindings = pdfium()?.bindings();
        let handle = self.document.handle();

        for index in (0..unsafe { bindings.FPDFDoc_GetAttachmentCount(handle) }).rev() {
            let existing = unsafe { bindings.FPDFDoc_GetAttachment(handle, index) };
            if !existing.is_null() && attachment_name(bindings, existing) == vault::ATTACHMENT {
                unsafe { bindings.FPDFDoc_DeleteAttachment(handle, index) };
            }
        }

        let mut name: Vec<u16> = vault::ATTACHMENT.encode_utf16().collect();
        name.push(0);
        let attachment =
            unsafe { bindings.FPDFDoc_AddAttachment(handle, name.as_ptr() as FPDF_WIDESTRING) };
        if attachment.is_null() {
            return Err(PdfError::Pdfium("the lock could not be attached".into()));
        }

        let written = unsafe {
            bindings.FPDFAttachment_SetFile(
                attachment,
                handle,
                bytes.as_ptr() as *const c_void,
                bytes.len() as c_ulong,
            )
        } != 0;
        if !written {
            return Err(PdfError::Pdfium("the lock could not be written".into()));
        }

        self.touch();
        // A locked document carries its original inside itself. Appending a
        // delta would leave the *unsealed* page in the file's earlier revision,
        // which is the same trap redaction has and the same answer.
        self.redacted = true;
        Ok(())
    }
}

/// Every path on this page that looks like a letter, matched against a
/// candidate face.
///
/// The shared core behind [`PdfiumPage::recognise_outlined`] and redaction's
/// use of the same matcher — both need exactly this, one to reassemble into
/// text, the other to decide what may be removed, and a second copy of the
/// forty-odd lines that read a path's segments is a second place for the two
/// to quietly stop agreeing about what a letter is.
///
/// Kept free rather than a method on `PdfiumDocument`, because it operates on
/// a `PdfPage` the caller already holds — redaction opens one, uses it here,
/// and drops it *before* opening the raw page handle it mutates. Two live
/// handles on the same page is a hazard this file already knows about
/// elsewhere: `set_text_run_styled`'s own comment is where that lesson was
/// paid for the first time.
/// Swept against `outlined.pdf` and a real system font, after
/// `Outline::distance_to` was fixed to stop scoring a letter against its own
/// mirror image as a mismatch — see that function's doc comment for what was
/// actually wrong. Below this, `identify`'s own ambiguity guard starts
/// refusing genuine matches whose runner-up happens to sit close by; above it,
/// the same guard's margin widens with the tolerance and starts refusing
/// *more* of them, not fewer — recognition measurably got worse, not better,
/// past this point. One page of one font is not a corpus, so this may need
/// revisiting once a larger one exists — see the module docs for what still
/// goes wrong even at this setting.
///
/// Shared at file scope, not a local inside [`identify_outlined_glyphs`],
/// because [`PdfiumPage::recognise_outlined_words`] needs the *same* value to
/// turn a match's distance into a confidence — a caller cannot honestly
/// rescale a score against a tolerance it does not know was used.
const OUTLINE_MATCH_TOLERANCE: f32 = 0.08;

/// The clusters a page's drawn shapes fall into, before any matching.
///
/// Split out from [`identify_outlined_glyphs`] so the two questions can be
/// asked separately: whether the letters were *found*, and whether they were
/// *recognised*. A page that reads back as single letters is one or the other,
/// and they want opposite fixes.
fn outlined_clusters(page: &PdfPage) -> Vec<crate::document::outlined::Cluster> {
    use crate::document::outlined::{build_outline, cluster, PathSegment, SegmentKind, SourcedContour};
    use pdfium_render::prelude::{PdfPathSegmentType, PdfPathSegments};

    let space = PageSpace::for_page(page, page.height().value);
    let (page_width, page_height) = (page.width().value, page.height().value);
    let mut contours: Vec<SourcedContour> = Vec::new();

    for (index, object) in page.objects().iter().enumerate() {
        let Some(path) = object.as_path_object() else { continue };
        let Ok(bounds) = object.bounds() else { continue };
        let width = (bounds.right().value - bounds.left().value).abs();
        let height = (bounds.top().value - bounds.bottom().value).abs();
        if !crate::document::classify::looks_like_type(
            width,
            height,
            path.segments().len() as usize,
            page_width,
            page_height,
        ) {
            continue;
        }
        let Ok(matrix) = object.matrix() else { continue };
        let mut segments: Vec<PathSegment> = Vec::new();
        for segment in path.segments().transform(matrix).iter() {
            let (x, y) = space.to_top_left(segment.x().value, segment.y().value);
            let kind = match segment.segment_type() {
                PdfPathSegmentType::MoveTo => SegmentKind::MoveTo,
                PdfPathSegmentType::LineTo => SegmentKind::LineTo,
                _ => SegmentKind::BezierTo,
            };
            segments.push(PathSegment { kind, x, y, close: segment.is_close() });
        }
        for contour in build_outline(&segments).contours {
            contours.push(SourcedContour { contour, object: index });
        }
    }
    cluster(contours)
}

fn identify_outlined_glyphs(
    page: &PdfPage,
    catalogue: &crate::document::glyphs::Catalogue,
) -> Vec<crate::document::outlined::Identified> {
    use crate::document::outlined::{build_outline, cluster, identify, PathSegment, SegmentKind, SourcedContour};
    use pdfium_render::prelude::{PdfPathSegmentType, PdfPathSegments};

    let space = PageSpace::for_page(page, page.height().value);
    let page_width = page.width().value;
    let page_height = page.height().value;

    let mut contours: Vec<SourcedContour> = Vec::new();

    for (index, object) in page.objects().iter().enumerate() {
        let Some(path) = object.as_path_object() else { continue };
        let Ok(bounds) = object.bounds() else { continue };
        let width = (bounds.right().value - bounds.left().value).abs();
        let height = (bounds.top().value - bounds.bottom().value).abs();
        let segment_count = path.segments().len() as usize;

        if !crate::document::classify::looks_like_type(
            width,
            height,
            segment_count,
            page_width,
            page_height,
        ) {
            continue;
        }

        let Ok(matrix) = object.matrix() else { continue };
        let mut segments: Vec<PathSegment> = Vec::new();
        for segment in path.segments().transform(matrix).iter() {
            let (raw_x, raw_y) = (segment.x().value, segment.y().value);
            let (x, y) = space.to_top_left(raw_x, raw_y);
            let kind = match segment.segment_type() {
                PdfPathSegmentType::MoveTo => SegmentKind::MoveTo,
                PdfPathSegmentType::LineTo => SegmentKind::LineTo,
                // `Unknown` cannot happen for a segment PDFium actually
                // handed back; treated as a line rather than dropped, so one
                // segment PDFium cannot name does not silently erase
                // everything drawn after it.
                PdfPathSegmentType::BezierTo | PdfPathSegmentType::Unknown => {
                    if segment.segment_type() == PdfPathSegmentType::Unknown {
                        SegmentKind::LineTo
                    } else {
                        SegmentKind::BezierTo
                    }
                }
            };
            segments.push(PathSegment { kind, x, y, close: segment.is_close() });
        }

        let outline = build_outline(&segments);
        for contour in outline.contours {
            contours.push(SourcedContour { contour, object: index });
        }
    }

    let clusters = cluster(contours);
    identify(&clusters, catalogue, OUTLINE_MATCH_TOLERANCE)
}

/// PDFium's `FPDFBitmap_BGRA`, which `pdfium-render` keeps in a private module.
///
/// Four bytes a pixel, blue first — the layout `FPDFBitmap_CreateEx` expects
/// when it is told this format.
const BITMAP_BGRA: c_int = 4;
/// `FPDFBitmap_Gray`: one byte a pixel.
const BITMAP_GRAY: c_int = 1;
/// `FPDFBitmap_BGR`: three bytes a pixel, blue first, no alpha.
const BITMAP_BGR: c_int = 2;
/// `FPDFBitmap_BGRx`: four bytes a pixel, blue first, the fourth unused.
const BITMAP_BGRX: c_int = 3;

/// A PDFium bitmap's pixels, whatever format it came back in, as straight
/// (not premultiplied) RGBA, tightly packed, top row first.
///
/// A bitmap PDFium hands back is padded to `stride` bytes a row — never
/// assumed equal to `width * bytes_per_pixel` — and may be any of the four
/// formats a page can hold, not only the BGRA this engine always writes:
/// **the file itself** decides that when it is saved, and a fully opaque
/// picture is free to come back as plain BGR with no alpha channel at all.
/// `None` for a format this does not know, or a stride too small for the
/// width it claims.
fn straight_rgba(buffer: &[u8], width: usize, height: usize, stride: usize, format: c_int) -> Option<Vec<u8>> {
    let bytes_per_pixel = match format {
        f if f == BITMAP_GRAY => 1,
        f if f == BITMAP_BGR => 3,
        f if f == BITMAP_BGRX || f == BITMAP_BGRA => 4,
        _ => return None,
    };
    if stride < width.checked_mul(bytes_per_pixel)? || buffer.len() < stride.checked_mul(height)? {
        return None;
    }

    let mut rgba = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
    for y in 0..height {
        let row = &buffer[y * stride..y * stride + width * bytes_per_pixel];
        let out = &mut rgba[y * width * 4..(y + 1) * width * 4];
        match bytes_per_pixel {
            1 => {
                for (px, &g) in out.chunks_exact_mut(4).zip(row) {
                    px.copy_from_slice(&[g, g, g, 255]);
                }
            }
            3 => {
                for (px, bgr) in out.chunks_exact_mut(4).zip(row.chunks_exact(3)) {
                    px.copy_from_slice(&[bgr[2], bgr[1], bgr[0], 255]);
                }
            }
            // BGRx and BGRA read alike here: `x` is unused padding PDFium
            // never draws with, so treating it as alpha 255 either way is
            // exactly what an opaque BGRx pixel already means.
            4 => {
                for (px, bgra) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                    let alpha = if format == BITMAP_BGRX { 255 } else { bgra[3] };
                    px.copy_from_slice(&[bgra[2], bgra[1], bgra[0], alpha]);
                }
            }
            _ => unreachable!("matched above"),
        }
    }
    Some(rgba)
}

/// A slice of bytes, as the reader `FPDFImageObj_LoadJpegFileInline` wants.
///
/// PDFium reads a JPEG through a callback rather than taking a buffer, so the
/// bytes stay where they are and this hands over a pointer to them. **Both the
/// slice and the `&[u8]` binding it is taken from must outlive the call** —
/// hence the reference-to-a-reference, which is what `m_Param` carries.
fn jpeg_access(bytes: &&[u8]) -> pdfium_render::prelude::FPDF_FILEACCESS {
    unsafe extern "C" fn read_block(
        param: *mut c_void,
        position: c_ulong,
        buffer: *mut u8,
        size: c_ulong,
    ) -> c_int {
        // Safety: `param` is the `&[u8]` handed in below, alive for the whole
        // call. PDFium promises position and size stay inside the length it was
        // given; the check makes that a refusal rather than a trusted claim.
        let bytes: &[u8] = unsafe { *(param as *const &[u8]) };
        let (start, len) = (position as usize, size as usize);
        let Some(end) = start.checked_add(len).filter(|e| *e <= bytes.len()) else {
            return 0;
        };
        unsafe { std::ptr::copy_nonoverlapping(bytes[start..end].as_ptr(), buffer, len) };
        1
    }

    pdfium_render::prelude::FPDF_FILEACCESS {
        m_FileLen: bytes.len() as c_ulong,
        m_GetBlock: Some(read_block),
        m_Param: (bytes as *const &[u8]) as *mut c_void,
    }
}

/// The codes an operator draws, as numbers.
fn codes_of(operation: &crate::pdf::content::Operation, width: usize) -> Vec<u32> {
    let mut out = Vec::new();
    for piece in crate::pdf::content::pieces(operation) {
        if let crate::pdf::content::Piece::Codes(raw) = piece {
            for chunk in raw.chunks(width.max(1)) {
                out.push(chunk.iter().fold(0u32, |code, byte| code << 8 | u32::from(*byte)));
            }
        }
    }
    out
}

/// Which code produced each character of a run.
///
/// # Why counting is not enough
///
/// A code and a character are the same thing only in the easy case. A ligature
/// code spells two characters — measured on a catalogue page, `02DB` spells
/// `"fl"` — and a code can spell something PDFium leaves out of its extracted
/// text entirely, which is how an operator comes to draw 44 codes for 42
/// characters. Either way, past that point a character index and a code index
/// are different numbers, and cutting at one while meaning the other lands on
/// the wrong glyphs.
///
/// So the font's own `/ToUnicode` table is walked against the text PDFium
/// reported: each code either spells the characters at the current position, or
/// spells nothing that survived. `None` when the two cannot be reconciled —
/// which the caller must treat as "do not cut here" rather than as an offset to
/// guess at.
fn align_codes(spellings: &[Option<String>], text: &str) -> Option<Vec<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let mut owner: Vec<usize> = Vec::with_capacity(chars.len());
    let mut at = 0usize;

    for (index, spelling) in spellings.iter().enumerate() {
        // A code the table does not mention is skipped, not fatal. CID fonts
        // routinely leave one out — measured, nine of twelve fallbacks on a
        // real catalogue were a `C2_0` font whose table omits six of its
        // thirty-one codes — and refusing on the first of them threw away the
        // whole alignment.
        //
        // Skipping is safe because of the check at the end: if such a code did
        // produce a character, the sequential match stalls on it and the
        // characters stop adding up, which is refused there.
        let Some(spelling) = spelling else { continue };
        let wanted: Vec<char> = spelling.chars().collect();
        if wanted.is_empty() || at + wanted.len() > chars.len() {
            continue;
        }
        if chars[at..at + wanted.len()] == wanted[..] {
            owner.extend(std::iter::repeat_n(index, wanted.len()));
            at += wanted.len();
        }
        // Otherwise this code left nothing in the extracted text — a soft
        // hyphen, or anything else PDFium drops — and owns no characters.
    }
    // Every character has to be accounted for — except trailing whitespace,
    // which PDFium adds on its own.
    //
    // Measured on a real catalogue: the font's table spells
    // `"Color Temperature        6500 k"` where PDFium reports
    // `"Color Temperature 6500 k "` — it collapses the run of spaces (which the
    // loop above already skips past) and appends one of its own at the end.
    // That last space belongs to no code, and refusing on account of it threw
    // away nine of the twelve remaining fallbacks on that document.
    //
    // Such characters are marked as owned by nothing, so a selection covering
    // one removes no code rather than the wrong one.
    while at < chars.len() && chars[at].is_whitespace() {
        owner.push(usize::MAX);
        at += 1;
    }
    (at == chars.len()).then_some(owner)
}

/// The operator nearest a point, when one is near enough to be it.
///
/// Four points: close enough to survive the rounding between PDFium's idea of a
/// run's box and the stream's own numbers, and far short of the gap to the next
/// line of text on any page anybody sets.
fn nearest_placed(
    placed: &[crate::pdf::content::Placed],
    x: f32,
    y: f32,
) -> Option<(usize, f32)> {
    placed
        .iter()
        .enumerate()
        .map(|(at, p)| (at, ((p.origin.x - x).powi(2) + (p.origin.y - y).powi(2)).sqrt()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d <= 4.0)
}

/// The operator for a run, matched by counting rather than by position.
///
/// **Only where the page has proved that counting works.** PDFium reports a
/// page's text objects in the order the content stream draws them, so the *n*th
/// run should be the *n*th operator that shows text — but "should" is not a
/// thing to edit somebody's document on. So the claim is checked against the
/// runs that can be matched the reliable way:
///
/// - there are exactly as many operators as runs,
/// - every run that *can* be placed by position is at its own index,
/// - and at least half of them could be, so a page of two runs cannot prove
///   anything by accident.
///
/// Fail any of those and this returns `None`, which the caller reports as a run
/// it cannot edit — the same answer it gave before, for the same reason: a
/// guess here rewrites the wrong words.
fn placed_in_order<'a>(
    runs: &[crate::document::TextRun],
    placed: &'a [crate::pdf::content::Placed],
    height: f32,
    wanted: usize,
) -> Option<&'a crate::pdf::content::Placed> {
    if runs.len() != placed.len() || wanted >= placed.len() {
        return None;
    }
    let mut confirmed = 0usize;
    for (index, run) in runs.iter().enumerate() {
        match nearest_placed(placed, run.rect.left, height - run.rect.bottom) {
            Some((at, _)) if at == index => confirmed += 1,
            // One run placed somewhere other than its own index and the order
            // is not the order. Nothing here is worth a wrong edit.
            Some(_) => return None,
            None => {}
        }
    }
    (confirmed * 2 >= runs.len()).then(|| &placed[wanted])
}

/// A retyped line that is to span a width, with what it takes to write it again
/// once the width of its plain version is known — see
/// [`PdfiumDocument::set_runs_in_stream`]'s `stretch`.
///
/// **How a line is spread.** The words of a justified line are spaced apart to
/// fill it, and retyping it writes plain words that come out a few points short.
/// What put the producer's spacing there is gone; what puts it back is a `TJ`
/// spacing number after each of the line's gaps — the spaces between its first
/// and last letters, or, for a line of one word, the places between its letters.
/// All the gaps get the same share of the difference, as a word-spacing
/// operator would give them, and **a `TJ` number belongs to the operator it is
/// written in**: nothing is left in force for the lines after it, which `Tw`
/// and `Tc` (graphics state, to be set and set back) would need — and a
/// composite font has no single-byte space for `Tw` to find.
struct Stretch {
    /// Where its replacement sits in the batch's spans.
    span: usize,
    /// The run (its object number before the edit) and the width, in points from
    /// its left origin, that it is to span.
    object: usize,
    target: f32,
    /// The operator it is written into (an index into the stream's operations),
    /// the codes of it that the new words replace, and the new words as codes and
    /// as text — one code to a character, `encoded.len() / text.chars().count()`
    /// bytes each.
    operation: usize,
    range: std::ops::Range<usize>,
    encoded: Vec<u8>,
    text: String,
    /// The font resource it is drawn in, the `Tf` size it is drawn at and the
    /// width in bytes of a code of the operator's own font.
    drawing: Vec<u8>,
    size: f32,
    width: usize,
    /// What follows the operator to put the page's own `Tf` back, and what goes
    /// before it to say a quote operator's move to the next line.
    after: Vec<u8>,
    before: Vec<u8>,
    /// How many points one `TJ` number of 1 moves the pen along the line: the
    /// size, the magnification of the text matrix and the page's transform, and
    /// the horizontal scaling, over a thousand.
    points_per_unit: f32,
}

impl Stretch {
    /// The operator's replacement for a line whose plain version is `plain_width`
    /// points wide, written so that it comes out [`Self::target`] wide.
    ///
    /// Refused — with `InvalidArgument`, and nothing written — when the gaps
    /// would have to close until the words touch (a line far too long for the
    /// width, a third of an em a gap and more), when a single word would have to
    /// open beyond what a word can bear (more than a third of an em between
    /// letters), and when the line is of a single character, which has no gap at
    /// all.
    fn spread(&self, operation: &crate::pdf::content::Operation, plain_width: f32) -> Result<Vec<u8>> {
        use crate::pdf::content::{self, Piece};

        let delta = self.target - plain_width;
        let chars: Vec<char> = self.text.chars().collect();
        // Which gaps the difference is shared among: the characters after which
        // a spacing number goes.
        let first = chars.iter().position(|c| *c != ' ');
        let last = chars.iter().rposition(|c| *c != ' ');
        let gaps: Vec<usize> = match (first, last) {
            (Some(first), Some(last)) => {
                let spaces: Vec<usize> = (first + 1..last).filter(|j| chars[*j] == ' ').collect();
                if spaces.is_empty() { (first..last).collect() } else { spaces }
            }
            _ => Vec::new(),
        };
        let between_words = gaps.first().is_some_and(|j| chars[*j] == ' ');
        let em = self.points_per_unit * 1000.0;

        // Where the plain line already spans the width, to a hundredth of a
        // point, it is written as it is.
        let kern = if delta.abs() < 0.005 {
            0.0
        } else {
            if gaps.is_empty() {
                return Err(PdfError::InvalidArgument(
                    "a line of one character has no gap to open, so it cannot be stretched to a width".into(),
                ));
            }
            let each = delta / gaps.len() as f32;
            if between_words && each < -0.35 * em {
                return Err(PdfError::InvalidArgument(format!(
                    "this line is {:.1} pt wider than the {:.1} pt it is to fit, and closing its {} gaps by that much \
                     would put its words on top of each other",
                    -delta, self.target, gaps.len()
                )));
            }
            if !between_words && (each < -0.1 * em || each > 0.35 * em) {
                return Err(PdfError::InvalidArgument(format!(
                    "this line is a single word, {:.1} pt wide, and cannot be spread to {:.1} pt without pulling its \
                     letters apart or onto each other",
                    plain_width, self.target
                )));
            }
            -each / self.points_per_unit
        };

        let per_char = if chars.is_empty() { 0 } else { self.encoded.len() / chars.len() };
        if chars.is_empty() || per_char == 0 || per_char * chars.len() != self.encoded.len() {
            return Err(PdfError::InvalidArgument(
                "this line's codes cannot be told from its characters, so it cannot be stretched to a width".into(),
            ));
        }
        let mut pieces: Vec<Piece> = Vec::new();
        let mut run: Vec<u8> = Vec::new();
        for (j, bytes) in self.encoded.chunks(per_char).enumerate() {
            run.extend_from_slice(bytes);
            if kern != 0.0 && gaps.binary_search(&j).is_ok() {
                pieces.push(Piece::Codes(std::mem::take(&mut run)));
                pieces.push(Piece::Kern(kern));
            }
        }
        if !run.is_empty() {
            pieces.push(Piece::Codes(run));
        }

        let mut replacement =
            content::replacing_pieces(operation, self.range.clone(), &pieces, &self.drawing, self.size, self.width);
        replacement.extend_from_slice(&self.after);
        if !self.before.is_empty() {
            replacement = [self.before.as_slice(), b"\n", replacement.as_slice()].concat();
        }
        Ok(replacement)
    }
}

/// Whether a font writes top to bottom: a composite font with a vertical CMap
/// (`Identity-V`, or any other ending in `-V`). A `TJ` number in one moves the
/// pen *down* the line, so a box's width says nothing about how far it has been
/// spread.
fn font_is_vertical(file: &crate::pdf::File<'_>, fonts: &crate::pdf::Dict, name: &[u8]) -> bool {
    use crate::pdf::Object;
    let Some(font) = fonts.get(name).and_then(|f| file.resolve(f).ok()) else { return false };
    let Some(dict) = font.as_dict() else { return false };
    dict.get(b"Subtype").and_then(Object::as_name) == Some(b"Type0")
        && dict.get(b"Encoding").and_then(Object::as_name).is_some_and(|n| n.ends_with(b"-V"))
}

/// A run written in a font that is not the one it was drawn in.
struct Swapped {
    /// The resource name to select, without the slash.
    resource: Vec<u8>,
    /// What to call the face when telling somebody the look changed.
    face: String,
    /// Objects to add to the file. Empty when the font was already there.
    added: Vec<(u32, Vec<u8>)>,
    /// The page object rewritten to name the new font, where that was needed.
    page: Option<(u32, Vec<u8>)>,
}

/// What a single-byte font with no `/ToUnicode` is taken to spell: printable
/// ASCII, one code each.
fn assumed_ascii() -> crate::pdf::cmap::ToUnicode {
    (0x20u32..0x7F)
        .filter_map(|code| char::from_u32(code).map(|c| (code, c.to_string())))
        .collect()
}

/// `unicode` read backwards — which code spells each character — **less the
/// characters the font's subset has no outline for.**
///
/// `ink` is [`crate::pdf::subset::drawable_ascii`]'s answer, `None` meaning it
/// could not tell and nothing is left out. A character that is already drawn
/// in `drawn` is kept whatever `ink` says: it has an outline by being on the
/// page, and a name this could not match must not make a run unable to be
/// retyped with its own words.
fn spelling_codes(
    unicode: &crate::pdf::cmap::ToUnicode,
    ink: Option<&std::collections::BTreeSet<u32>>,
    drawn: &str,
) -> std::collections::BTreeMap<String, u32> {
    let mut reverse: std::collections::BTreeMap<String, u32> = Default::default();
    for (code, spelling) in unicode {
        if ink.is_some_and(|ink| !ink.contains(code) && !drawn.contains(spelling.as_str())) {
            continue;
        }
        reverse.entry(spelling.clone()).or_insert(*code);
    }
    reverse
}

/// The codes that spell some words in a font, or `None` if it cannot.
fn encode_with(
    reverse: &std::collections::BTreeMap<String, u32>,
    text: &str,
    width: usize,
) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * width);
    for character in text.chars() {
        let code = *reverse.get(&character.to_string())?;
        for shift in (0..width).rev() {
            out.push((code >> (shift * 8)) as u8);
        }
    }
    Some(out)
}

/// The words to encode: what was asked for, less trailing whitespace this font
/// has no glyph for.
///
/// PDFium appends a space of its own to a run's extracted text, and plenty of
/// fonts have no space glyph at all — a space is made by moving the pen, not by
/// drawing one. Between them, that made some runs impossible to retype *with
/// their own words*: measured on a real document, four of twenty-nine, and the
/// four were simply the ones whose text PDFium had punctuated for them.
///
/// Only the tail, and only whitespace. A space in the middle of a sentence is a
/// gap somebody asked for, and quietly closing it would change the words.
fn encodable(text: &str, reverse: &std::collections::BTreeMap<String, u32>) -> String {
    let mut kept = text.to_string();
    while kept
        .chars()
        .next_back()
        .is_some_and(|c| c.is_whitespace() && !reverse.contains_key(&c.to_string()))
    {
        kept.pop();
    }
    kept
}

/// The characters a run's font can actually draw, for saying so.
///
/// Listed where the list is short enough to read and counted where it is not —
/// a hundred characters in an error message is a wall, not an answer.
fn typeable(reverse: &std::collections::BTreeMap<String, u32>) -> String {
    let mut characters: Vec<&str> = reverse
        .keys()
        .filter(|k| !k.trim().is_empty())
        .map(String::as_str)
        .collect();
    characters.sort_unstable();
    if characters.is_empty() {
        return "nothing this can read".into();
    }
    if characters.len() > 48 {
        return format!("{} different characters, none of them that one", characters.len());
    }
    characters.join("")
}

/// The codes a run occupies, from the alignment `align_codes` produced.
///
/// **The sentinel is not a code.** `align_codes` marks a character that no code
/// produced — PDFium's own appended trailing space — as owned by `usize::MAX`,
/// and a run whose extracted text ends in one carries that as its *last* entry.
/// Read literally it made the replaced range `0..usize::MAX + 1`, which in a
/// release build wraps to `0..0`: replace nothing, insert everything. The old
/// words stayed and the new ones were written after them.
///
/// Reported from use with a screenshot — a page footer reading
/// `HUE . SATURATION . INTENSITYHUE . SATURATION . IN…` running off the edge of
/// the page. The locking path filters the sentinel out; this one did not.
///
/// `None` when no character belongs to any code, which is not something to
/// guess a range for.
fn owned_codes(owner: &[usize]) -> Option<std::ops::Range<usize>> {
    let mut real = owner.iter().copied().filter(|code| *code != usize::MAX);
    let first = real.next()?;
    let last = real.last().unwrap_or(first);
    Some(first..last.max(first) + 1)
}

/// How many bytes make one character code in a font.
///
/// One for a simple font. Two for a `Type0` with an identity encoding, which is
/// what a subset CID font in a real catalogue uses. **Anything else is `None`**
/// — a `Type0` behind a named or embedded CMap maps codes to glyphs through a
/// table this does not read, and slicing its string at a guessed boundary would
/// cut a glyph in half.
fn code_width(
    file: &crate::pdf::File<'_>,
    fonts: &crate::pdf::Dict,
    name: &[u8],
) -> Option<usize> {
    use crate::pdf::Object;
    let font = file.resolve(fonts.get(name)?).ok()?;
    match font.as_dict()?.get(b"Subtype").and_then(Object::as_name) {
        Some(b"Type0") => match font.as_dict()?.get(b"Encoding").and_then(Object::as_name) {
            Some(b"Identity-H" | b"Identity-V") => Some(2),
            _ => None,
        },
        Some(_) => Some(1),
        None => None,
    }
}

/// The transform in force at one operation, walked from the start of a stream.
///
/// The same state machine `content::placed` runs for text, kept here because
/// this needs only the graphics transform and needs it for an operator that
/// draws no text.
fn ctm_at(
    operations: &[crate::pdf::content::Operation],
    target: &crate::pdf::content::Operation,
) -> [f32; 6] {
    fn multiply(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
        [
            a[0] * b[0] + a[1] * b[2],
            a[0] * b[1] + a[1] * b[3],
            a[2] * b[0] + a[3] * b[2],
            a[2] * b[1] + a[3] * b[3],
            a[4] * b[0] + a[5] * b[2] + b[4],
            a[4] * b[1] + a[5] * b[3] + b[5],
        ]
    }
    const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

    let mut ctm = IDENTITY;
    let mut stack: Vec<[f32; 6]> = Vec::new();
    for operation in operations {
        if operation.span == target.span {
            break;
        }
        match operation.operator.as_slice() {
            b"q" => stack.push(ctm),
            b"Q" => ctm = stack.pop().unwrap_or(IDENTITY),
            b"cm" => {
                let numbers: Vec<f32> = operation
                    .operands
                    .iter()
                    .filter_map(|o| o.as_f64().map(|n| n as f32))
                    .collect();
                if let [a, b, c, d, e, f] = numbers[..] {
                    ctm = multiply([a, b, c, d, e, f], ctm);
                }
            }
            _ => {}
        }
    }
    ctm
}

/// The operators that draw ink strokes into a page.
///
/// Used to burn a placed signature into the page it sits on — see
/// [`crate::document::DocumentMut::apply_signatures`]. Wrapped in `q`/`Q` so
/// the colour and line width set here cannot leak into whatever the page draws
/// afterwards, and round caps and joins because a signature is written with a
/// nib rather than ruled.
fn ink_operators(
    strokes: &[Vec<Point>],
    colour: Color,
    width: f32,
    page_height: f32,
) -> Vec<u8> {
    let mut body = String::new();
    for stroke in strokes {
        let mut points = stroke.iter();
        let Some(first) = points.next() else { continue };
        // A single point draws nothing, the same as in the annotation.
        if stroke.len() < 2 {
            continue;
        }
        // The points arrive top-left down; a content stream works bottom-left up.
        body.push_str(&format!("{} {} m\n", first.x, page_height - first.y));
        for point in points {
            body.push_str(&format!("{} {} l\n", point.x, page_height - point.y));
        }
        body.push_str("S\n");
    }
    if body.is_empty() {
        return Vec::new();
    }

    let (r, g, b) = (
        f32::from(colour.r) / 255.0,
        f32::from(colour.g) / 255.0,
        f32::from(colour.b) / 255.0,
    );
    format!("\nq\n{r} {g} {b} RG\n{} w\n1 J\n1 j\n{body}Q\n", width.max(0.1)).into_bytes()
}

/// The operators that draw a placed picture: position it with `cm`, then
/// `Do` the XObject named `resource` — which the caller has already put in
/// the page's own `/Resources/XObject`.
///
/// Same coordinate convention as [`ink_operators`] and [`mark`]: the plain
/// `page_height - y` flip, no crop-box offset — this is content-stream space,
/// which every other painter in this file already treats as the media box.
///
/// Not `image_operators` — that name is taken, by the unrelated function
/// below that reads `Do` operators back *out* of a content stream to find
/// which images a page already draws.
///
/// `rotation_degrees` reuses [`image_placement_matrix`]'s own formula — the
/// same matrix a placed, unrotated-or-not signature is shown at while it is
/// still an annotation is what gets burnt into the page here, not a second,
/// independently-derived one that could drift from it.
fn place_image_operators(resource: &[u8], rect: Rect, page_height: f32, rotation_degrees: f32) -> Vec<u8> {
    let pdf_rect = FS_RECTF {
        left: rect.left,
        right: rect.right,
        top: page_height - rect.top,
        bottom: page_height - rect.bottom,
    };
    let m = image_placement_matrix(&pdf_rect, rotation_degrees);
    let mut out = format!("
q
{} {} {} {} {} {} cm
/", m.a, m.b, m.c, m.d, m.e, m.f).into_bytes();
    out.extend_from_slice(resource);
    out.extend_from_slice(b" Do
Q
");
    out
}

/// A picture as an Image XObject: `/DeviceRGB`, one filter,
/// FlateDecode-compressed — opaque unless `smask` names another XObject to
/// pair it with.
///
/// **Why applying can be transparent when placing cannot.** `Annotation::
/// Image`'s own doc explains the wall: the convenience PDFium object API
/// this engine *places* a picture through does not carry alpha even in
/// memory, proved by a dedicated probe — so the interactive annotation
/// somebody positions is opaque, full stop, and nothing here changes that.
/// This function is different: it hand-writes the PDF bytes directly rather
/// than asking PDFium to hold a bitmap, so it is not bound by that API's
/// limit. `smask`, when given, is the object number of a `/DeviceGray`
/// XObject the same shape as this one — see [`alpha_smask_xobject`] — built
/// from the picture's *original* alpha, kept outside PDFium's lossy round
/// trip for exactly this moment (`PdfiumDocument::image_alpha`). The alpha
/// byte in `rgba` itself is still dropped here either way: colour and
/// opacity are two separate image objects in PDF, and the mask carries the
/// second.
fn image_xobject(rgba: &[u8], width: u32, height: u32, smask: Option<u32>) -> Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    let packed = crate::pdf::content::encode(&rgb)?;
    let mut dict = crate::pdf::Dict(Vec::new());
    dict.set(b"Type", crate::pdf::Object::Name(b"XObject".to_vec()));
    dict.set(b"Subtype", crate::pdf::Object::Name(b"Image".to_vec()));
    dict.set(b"Width", crate::pdf::Object::Number(width.to_string().into_bytes()));
    dict.set(b"Height", crate::pdf::Object::Number(height.to_string().into_bytes()));
    dict.set(b"ColorSpace", crate::pdf::Object::Name(b"DeviceRGB".to_vec()));
    dict.set(b"BitsPerComponent", crate::pdf::Object::Number(b"8".to_vec()));
    dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
    if let Some(number) = smask {
        dict.set(b"SMask", crate::pdf::Object::Reference(number, 0));
    }
    Ok(crate::pdf::write_stream(&dict, &packed))
}

/// A picture's alpha channel alone, as its own `/DeviceGray` Image XObject —
/// the companion `/SMask` a colour [`image_xobject`] references to be
/// transparent where its original pixels were. 255 (the channel's own
/// scale) is fully opaque, matching the convention PDF's soft masks use.
fn alpha_smask_xobject(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let alpha: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
    let packed = crate::pdf::content::encode(&alpha)?;
    let mut dict = crate::pdf::Dict(Vec::new());
    dict.set(b"Type", crate::pdf::Object::Name(b"XObject".to_vec()));
    dict.set(b"Subtype", crate::pdf::Object::Name(b"Image".to_vec()));
    dict.set(b"Width", crate::pdf::Object::Number(width.to_string().into_bytes()));
    dict.set(b"Height", crate::pdf::Object::Number(height.to_string().into_bytes()));
    dict.set(b"ColorSpace", crate::pdf::Object::Name(b"DeviceGray".to_vec()));
    dict.set(b"BitsPerComponent", crate::pdf::Object::Number(b"8".to_vec()));
    dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
    Ok(crate::pdf::write_stream(&dict, &packed))
}

/// The operators that draw a tick, a cross or a dot.
///
/// Strokes rather than glyphs, so no font is needed and every reader shows the
/// same shape — see [`crate::document::DocumentMut::stamp_mark`]. Wrapped in
/// `q`/`Q` so the line width and colour set here cannot leak into whatever the
/// page draws afterwards.
fn fill_mark(
    mark: crate::document::FillMark,
    at: Point,
    size: f32,
    page_height: f32,
) -> Vec<u8> {
    use crate::document::FillMark;

    // The point arrives top-left down; a content stream works bottom-left up.
    let (x, y) = (at.x, page_height - at.y);
    let half = size / 2.0;
    // Thick enough to read at print size, thin enough not to fill the box.
    let weight = (size * 0.12).max(0.6);

    let body = match mark {
        // Down to the left foot, then up to the right — the way it is written.
        FillMark::Tick => format!(
            "{} {} m\n{} {} l\n{} {} l\nS\n",
            x - half,
            y,
            x - half * 0.25,
            y - half * 0.8,
            x + half,
            y + half * 0.9
        ),
        FillMark::Cross => format!(
            "{} {} m\n{} {} l\n{} {} m\n{} {} l\nS\n",
            x - half,
            y + half,
            x + half,
            y - half,
            x + half,
            y + half,
            x - half,
            y - half
        ),
        // A filled circle, drawn as four Béziers — the constant is the usual
        // one for approximating a quarter circle with a cubic.
        FillMark::Dot => {
            let k = half * 0.5523;
            format!(
                "{} {} m\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n\
                 {} {} {} {} {} {} c\n{} {} {} {} {} {} c\nf\n",
                x + half, y,
                x + half, y + k, x + k, y + half, x, y + half,
                x - k, y + half, x - half, y + k, x - half, y,
                x - half, y - k, x - k, y - half, x, y - half,
                x + k, y - half, x + half, y - k, x + half, y,
            )
        }
    };

    format!("\nq\n{FILL_INK} RG\n{FILL_INK} rg\n{weight} w\n1 J\n1 j\n{body}Q\n").into_bytes()
}

/// The ink every fill-and-sign mark is written in.
///
/// Near-black rather than black, which is what a pen looks like against print.
/// Shared by the tick, the cross, the dot and the box so that a form filled in
/// with two of them does not look filled in by two people.
const FILL_INK: &str = "0.06 0.06 0.09";

/// The operators that rule a line.
///
/// A constant weight, unlike the box: a line's thickness is the pen, and
/// scaling it with the length would draw a long strike-through fatter than a
/// short one. Round caps, so a short line is a stroke rather than a stub.
fn fill_line(from: Point, to: Point, page_height: f32) -> Vec<u8> {
    // The points arrive top-left down; a content stream works bottom-left up.
    format!(
        "\nq\n{FILL_INK} RG\n{FILL_LINE_WEIGHT} w\n1 J\n{} {} m\n{} {} l\nS\nQ\n",
        from.x,
        page_height - from.y,
        to.x,
        page_height - to.y
    )
    .into_bytes()
}

/// How thick a ruled line is, in points.
///
/// The middle of the range the box uses, which is what a pen looks like on a
/// printed form.
const FILL_LINE_WEIGHT: f32 = 1.2;

/// The operators that draw a box around something.
///
/// An outline, not a fill: this is the mark somebody puts around a field they
/// are answering, and a filled one would hide the answer. See
/// [`crate::document::DocumentMut::stamp_box`].
fn fill_box(area: Rect, page_height: f32) -> Vec<u8> {
    // The area arrives top-left down; a content stream works bottom-left up.
    let (bottom, top) = (page_height - area.bottom, page_height - area.top);
    let (width, height) = (area.right - area.left, top - bottom);
    // Scaled a little with the box, so a small one is not a blob and a large
    // one is not a hairline, and bounded at both ends.
    let weight = (width.min(height) * 0.04).clamp(0.8, 2.0);

    format!(
        "\nq\n{FILL_INK} RG\n{weight} w\n1 j\n{} {} {width} {height} re\nS\nQ\n",
        area.left, bottom
    )
    .into_bytes()
}

/// The operators that paint a redaction's mark over its area.
///
/// Appended after everything else, so it covers what it is over. Wrapped in
/// `q`/`Q` so the colour it sets does not leak into whatever a later stream
/// draws — the page's own operators are untouched, and must stay that way.
/// Where a wrapping edit goes for one object — see
/// `PdfiumDocument::object_wrap_site`.
struct WrapSite {
    /// The operations that are the object, first to one past the last.
    span: std::ops::Range<usize>,
    /// Whether that span is already `q … Q` of its own, so an edit can be
    /// written just inside its `q`; otherwise the span gets a scope around it.
    own_scope: bool,
    /// The transform in force where the edit is written.
    ctm: [f32; 6],
}

/// One thing drawn on a page, as the move guard sees it.
#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    kind: i32,
    rect: Rect,
    fill: Option<(u32, u32, u32, u32)>,
    stroke: Option<(u32, u32, u32, u32)>,
}

/// The `q`/`Q` scope that is one object's **frame**: the innermost scope around
/// it that draws nothing else.
///
/// **How a placed picture is actually written.** A design program wraps each
/// one as `q <its bounds> re W n q cm /Im Do Q Q` — a clip exactly the size of
/// the frame, then the picture inside it. Moving the `Do` alone slides the
/// picture out of its own clip, and past the frame's edge it is cut to
/// nothing: reported from use, with a screenshot, as a picture that vanished
/// when moved and left a grey block where it was — the placeholder rectangle
/// the same program paints under every photograph.
///
/// So when the innermost scope holds only this object and clip-setting paths,
/// the scope *is* the object's frame, and a move goes just inside its `q` —
/// where it carries the clip and the object together. Returns the `q` and the
/// `Q`. `None` where the scope also draws something else, or there is none.
fn frame_scope(
    operations: &[crate::pdf::content::Operation],
    object: std::ops::Range<usize>,
) -> Option<(usize, usize)> {
    /// The `q` that opens the innermost scope around `before`, if any.
    fn opening(operations: &[crate::pdf::content::Operation], before: usize) -> Option<usize> {
        let mut depth = 0i32;
        for index in (0..before).rev() {
            match operations[index].operator.as_slice() {
                b"Q" => depth += 1,
                b"q" => {
                    if depth == 0 {
                        return Some(index);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        None
    }
    /// The `Q` that closes the scope opened at `open`.
    fn closing(operations: &[crate::pdf::content::Operation], open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for (index, operation) in operations.iter().enumerate().skip(open + 1) {
            match operation.operator.as_slice() {
                b"q" => depth += 1,
                b"Q" => {
                    if depth == 0 {
                        return Some(index);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        None
    }

    // **Outward, as far as nothing else is drawn.** A placed picture sits in
    // two scopes — `q clip q cm Do Q Q` — and the innermost holds only the
    // `cm` and the `Do`. Stopping there moves the picture and leaves the clip
    // behind, which is exactly the vanishing this exists to prevent. So each
    // enclosing scope is tried in turn and the widest one that still draws
    // only this object is the frame.
    let mut frame = None;
    let mut from = object.start;
    while let Some(open) = opening(operations, from) {
        let Some(close) = closing(operations, open) else { break };
        let draws_something_else =
            operations.iter().enumerate().take(close).skip(open + 1).any(|(index, operation)| {
                !object.contains(&index)
                    && (operation.shows_text()
                        || matches!(
                            operation.operator.as_slice(),
                            b"Do" | b"sh" | b"BI" | b"S" | b"s" | b"f" | b"F" | b"f*" | b"B"
                                | b"B*" | b"b" | b"b*"
                        ))
            });
        if draws_something_else {
            break;
        }
        frame = Some((open, close));
        from = open;
    }
    frame
}

/// A filled rectangle the size of a frame's clip, painted immediately before
/// the frame — the placeholder a design program puts under every placed
/// picture. Returns the index of its `re`.
///
/// Two things have to hold, and both are checked: the operation before the
/// frame's `q` fills a path, and that path is one `re` whose corners are
/// within a point of the first clipping `re` inside the frame.
fn placeholder_before(operations: &[crate::pdf::content::Operation], open: usize) -> Option<usize> {
    // Two shapes the same placeholder takes. Written by the design program it
    // is bare — `re f` straight before the frame. After PDFium has re-emitted
    // the page (a lock's fallback does that) it comes back wrapped in a scope
    // of its own with its colour inside: `q cs sc gs re f Q`. Either way the
    // fill is the last drawing before the frame and the `re` is the only path.
    let before = open.checked_sub(1)?;
    let (start, fill) = if operations[before].operator == b"Q" {
        // Back to the `q` that this `Q` closes.
        let mut depth = 0i32;
        let mut index = before;
        let opening = loop {
            if index == 0 {
                return None;
            }
            index -= 1;
            match operations[index].operator.as_slice() {
                b"Q" => depth += 1,
                b"q" => {
                    if depth == 0 {
                        break index;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        };
        // Inside: one `re`, one fill, and nothing but state around them.
        let inner = &operations[opening + 1..before];
        let mut fill_at = None;
        for (offset, op) in inner.iter().enumerate() {
            match op.operator.as_slice() {
                b"re" => {}
                b"f" | b"F" | b"f*" => fill_at = Some(opening + 1 + offset),
                b"cs" | b"CS" | b"sc" | b"scn" | b"SC" | b"SCN" | b"g" | b"G" | b"rg" | b"RG"
                | b"k" | b"K" | b"gs" | b"w" | b"J" | b"j" | b"M" | b"d" | b"ri" | b"i" => {}
                _ => return None,
            }
        }
        (opening, fill_at?)
    } else {
        if !matches!(operations[before].operator.as_slice(), b"f" | b"F" | b"f*") {
            return None;
        }
        (before.checked_sub(1)?, before)
    };
    let rect = (start..fill).rev().find(|i| operations[*i].operator == b"re")?;
    if (start..fill).filter(|i| operations[*i].operator == b"re").count() != 1 {
        return None;
    }
    let clip = operations.iter().skip(open + 1).find(|o| o.operator == b"re")?;
    let corners = |o: &crate::pdf::content::Operation| -> Option<[f32; 4]> {
        let n: Vec<f32> = o.operands.iter().filter_map(|v| v.as_f64().map(|x| x as f32)).collect();
        (n.len() == 4).then(|| {
            // Normalised, because a negative height is how one of these
            // programs writes the same rectangle upside down.
            let (x, y, w, h) = (n[0], n[1], n[2], n[3]);
            [x.min(x + w), y.min(y + h), x.max(x + w), y.max(y + h)]
        })
    };
    let (a, b) = (corners(&operations[rect])?, corners(clip)?);
    let same = a.iter().zip(b.iter()).all(|(p, q)| (p - q).abs() <= 1.0);
    same.then_some(start)
}

/// Everything that is, to a reader, one placed picture: its placeholder
/// rectangle, its frame and the picture itself — as a range of operations.
///
/// **Why the unit and not the `Do`.** A design program writes a picture as
/// three things: a grey placeholder, a clip its own size, the picture inside
/// the clip. Sending the picture alone to the back put it *under its own
/// placeholder*, which painted grey over it — reported from use, for days, as
/// a grey layer that pictures kept going behind. Nobody looking at the page
/// means the third of those things when they point at a photograph.
fn picture_unit(
    operations: &[crate::pdf::content::Operation],
    at: usize,
) -> std::ops::Range<usize> {
    match frame_scope(operations, at..at + 1) {
        Some((open, close)) => {
            let start = placeholder_before(operations, open).unwrap_or(open);
            start..close + 1
        }
        None => at..at + 1,
    }
}

/// The operations that paint each path, in the order the page paints them, and
/// whether each one also sets a clip.
///
/// A path is a run of construction operators — `m`, `l`, `c`, `v`, `y`, `re`,
/// `h` — closed by a painting one. **Those ending in `n` paint nothing**: they
/// exist to set a clipping path, and PDFium does not report them as objects, so
/// leaving them out is what makes the *n*th of these the *n*th path object.
///
/// Measured on a real report rather than reasoned from the specification: ten
/// pages of ten agreeing, one of them with fifty painted paths and twenty-eight
/// clip-only ones.
fn path_operators(
    operations: &[crate::pdf::content::Operation],
) -> Vec<(std::ops::Range<usize>, bool)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut clips = false;

    for (index, operation) in operations.iter().enumerate() {
        match operation.operator.as_slice() {
            b"m" | b"re" => {
                if start.is_none() {
                    start = Some(index);
                    clips = false;
                }
            }
            b"l" | b"c" | b"v" | b"y" | b"h" => {}
            b"W" | b"W*" => clips = true,
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                if let Some(from) = start.take() {
                    // `n` paints nothing; it is here to close a clip.
                    if operation.operator.as_slice() != b"n" {
                        out.push((from..index + 1, clips));
                    }
                    clips = false;
                }
            }
            _ => {}
        }
    }
    out
}

/// The operations that draw a picture, in the order the page draws them.
///
/// A `Do` may draw a form rather than a picture, so the names are checked
/// against the page's own resources — which is why this takes the set rather
/// than working it out from the operator alone.
fn image_operators(
    operations: &[crate::pdf::content::Operation],
    images: &std::collections::BTreeSet<Vec<u8>>,
) -> Vec<usize> {
    operations
        .iter()
        .enumerate()
        .filter(|(_, op)| {
            op.operator == b"Do"
                && matches!(
                    op.operands.first(),
                    Some(crate::pdf::Object::Name(name)) if images.contains(name)
                )
        })
        .map(|(index, _)| index)
        .collect()
}

/// **Which operators draw a run**, and whether they can be lifted out.
///
/// Found the way the locking path finds them: the nearest placed origin to
/// where PDFium says the run starts, then everything continuing that line
/// within the run's own width. Proximity alone sweeps up a neighbouring column.
///
/// Returns the first and last operation inclusive, and **whether the line goes
/// on after them** — which is the difference between the two ways a run can be
/// moved, and a refusal for anything that lifts the run out of the stream.
///
/// Refuses outright where the operators cannot be isolated at all:
///
/// - another run drawn between them;
/// - a `BT` or `ET` inside, which means they are not one text object.
fn run_operators(
    run: &crate::document::TextRun,
    page_height: f32,
    placed: &[crate::pdf::content::Placed],
    operations: &[crate::pdf::content::Operation],
    codes_in: &dyn Fn(&crate::pdf::content::Placed) -> usize,
    order: Option<(usize, usize)>,
) -> Result<(usize, usize, bool)> {
    const NEAR: f32 = 4.0;
    // **The baseline origin, not the box's corner.** The box's bottom sits a
    // descender below the baseline and its left a side bearing past the pen,
    // and for large type either is more than four points — measured, a 30pt
    // line matched nothing. `TextRun::origin` is read from the text object's
    // own matrix and is exactly what `content::placed` computes.
    let (want_x, want_y) = (run.origin.x, page_height - run.origin.y);

    // **By count first, where the page proves it can be — as a picture and a
    // shape already are.** Reported from use: Edit Object refusing to delete a
    // word — "those words are drawn in a way this cannot follow". On the page
    // it was reported from, a third of the text could not be found by position,
    // and a measurable share of the rest was found *wrongly*.
    //
    // The cause is that `content::placed` reports where the last positioning
    // operator put the pen, not where the pen is — it cannot know how far the
    // glyphs advanced — so every show-text operator after the first in a line
    // is reported at the line's start. A run drawn as several pieces
    // (a hyphenated line's last syllable, a word set in its own fragment) is
    // therefore found nowhere, or, if the piece before it is only a few points
    // wide, found *at that piece*: measured, deleting "y repre-" deleted the
    // "il" before it.
    //
    // PDFium makes one text object per show-text operator, in the order the
    // stream draws them. So where there are as many of each, the *n*th object
    // is the *n*th operator — and that is checked against the stream before it
    // is trusted, rather than assumed (see [`order_agrees`]). Checked on the
    // page above: all 3,765 of its text objects agree, geometry and words, and
    // none disagrees.
    //
    // Anything that does not check out falls through to the position match
    // below, exactly as before.
    let by_order = placed_by_order(run, page_height, placed, order, codes_in);
    if let Some(start) = by_order {
        // One text object is one operator: there is nothing after it to collect.
        let at = start.origin.operation;
        return Ok((at, at, continues_after(operations, at)));
    }

    let start = placed
        .iter()
        .map(|p| {
            let d = ((p.origin.x - want_x).powi(2) + (p.origin.y - want_y).powi(2)).sqrt();
            (p, d)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d <= NEAR)
        .map(|(p, _)| p)
        .ok_or(PdfError::Unsupported(
            "those words are drawn in a way this cannot follow",
        ))?;

    // Everything continuing that line, from where the run begins.
    //
    // **The x bound cannot separate them.** `content::placed` reports where the
    // last positioning operator put the pen, not where the pen actually is —
    // so a line drawn as three `TJ`s with nothing between them reports all
    // three at the same place, and every test of position lets all three
    // through. What separates them is *how many codes each draws*: the run's
    // own operators are however many it takes to cover its characters, and the
    // rest of the line belongs to somebody else. Measured before this: dragging
    // `08-29 21:20` on a real page took the arrow and the date after it.
    let mut following: Vec<&crate::pdf::content::Placed> = placed
        .iter()
        .filter(|p| {
            p.line == start.line
                && p.origin.x >= start.origin.x - 0.5
                && p.origin.x <= run.rect.right + NEAR
        })
        .collect();
    following.sort_by_key(|p| p.origin.operation);

    let wanted = run.text.chars().count();
    let mut mine: Vec<usize> = Vec::new();
    let mut covered = 0usize;
    for p in &following {
        mine.push(p.origin.operation);
        covered += codes_in(p);
        if covered >= wanted {
            break;
        }
    }
    let (Some(&first), Some(&last)) = (mine.first(), mine.last()) else {
        return Err(PdfError::Unsupported(
            "those words are drawn in a way this cannot follow",
        ));
    };

    for (index, operation) in operations.iter().enumerate().take(last + 1).skip(first) {
        if operation.shows_text() && !mine.contains(&index) {
            return Err(PdfError::Unsupported("other words are drawn between these ones"));
        }
        if matches!(operation.operator.as_slice(), b"BT" | b"ET") {
            return Err(PdfError::Unsupported(
                "those words are split across more than one text object",
            ));
        }
    }

    Ok((first, last, continues_after(operations, last)))
}

/// **Does anything go on drawing this line?** The scan walks past the operators
/// that do not move the pen — `Tf`, a colour, a `gs` — and looks at the first
/// one that does. A show-text operator reached that way continues this very
/// line, and where the pen leaves off is decided by the advance of the glyphs in
/// between.
fn continues_after(operations: &[crate::pdf::content::Operation], last: usize) -> bool {
    for operation in operations.iter().skip(last + 1) {
        if matches!(
            operation.operator.as_slice(),
            b"Tm" | b"Td" | b"TD" | b"T*" | b"ET" | b"BT"
        ) {
            return false;
        }
        if operation.shows_text() {
            return true;
        }
    }
    false
}

/// Whether the operator found for a run *by count* is plausibly the one that
/// drew it — the check that stops "the *n*th object is the *n*th operator" being
/// taken on faith.
///
/// Two things, both read from the stream and neither from the count:
///
/// - **Where.** The run starts where the operator's own positioning put the
///   pen, or further along that same line in the direction the text runs — a
///   continuation is drawn *after* the pen advanced. Never a different line, and
///   never behind it. A point off the baseline by more than a couple of points
///   is another line's.
/// - **How much.** The operator draws at least as many codes as the run has
///   characters, less the one PDFium appends to some runs. *At least*, not
///   *exactly*: a run of spaces is one space in PDFium's text and as many codes
///   as the font drew — measured, `"        900mA"` read as `" 900mA"`, and an
///   exact match refused the right operator for it. A run PDFium extracted no
///   characters for (a font with no character map) is not held to it at all.
fn order_agrees(
    run: &crate::document::TextRun,
    operator: &crate::pdf::content::Placed,
    want_x: f32,
    want_y: f32,
    codes_in: &dyn Fn(&crate::pdf::content::Placed) -> usize,
) -> bool {
    let (dx, dy) = (want_x - operator.origin.x, want_y - operator.origin.y);
    let (ux, uy) = operator.axis;
    let along = dx * ux + dy * uy;
    let across = (dx * uy - dy * ux).abs();
    if along < -0.5 || across > 2.0 {
        return false;
    }
    let characters = run.text.chars().count();
    characters == 0 || codes_in(operator) + 1 >= characters
}

/// **The operator the stream confirms drew this run, found by count.**
///
/// PDFium makes one text object per show-text operator, in the order the stream
/// draws them, so where there are as many of each the *n*th object is the *n*th
/// operator. Unlike a lookup by position that finds a piece which continues the
/// one before it, whose operator has no position of its own — and which, where
/// the piece before it is narrow, finds *that* one instead, and edits the wrong
/// word without a sign of it.
///
/// Never taken on faith: [`order_agrees`] holds the operator to the stream.
/// `None` where the counts differ, where the object is not one of the page's
/// text objects, or where the stream does not agree — the caller has the
/// position match left to try. `order` is `(which text object this is, how many
/// the page has)`, counting **every** text object, whatever it draws: a lone thin
/// letter has a place in the order like any other.
fn placed_by_order<'a>(
    run: &crate::document::TextRun,
    page_height: f32,
    placed: &'a [crate::pdf::content::Placed],
    order: Option<(usize, usize)>,
    codes_in: &dyn Fn(&crate::pdf::content::Placed) -> usize,
) -> Option<&'a crate::pdf::content::Placed> {
    let (want_x, want_y) = (run.origin.x, page_height - run.origin.y);
    order
        .filter(|&(_, total)| total == placed.len())
        .and_then(|(which, _)| placed.get(which))
        .filter(|p| order_agrees(run, p, want_x, want_y, codes_in))
}

/// How many codes a show-text operator draws, by the width its font gives them —
/// the length [`order_agrees`] holds an operator to.
fn codes_drawn(
    file: &crate::pdf::File<'_>,
    fonts: Option<&crate::pdf::Dict>,
    operations: &[crate::pdf::content::Operation],
    operator: &crate::pdf::content::Placed,
) -> usize {
    use crate::pdf::content;
    let width = operator
        .font
        .as_ref()
        .zip(fonts)
        .and_then(|(name, dict)| code_width(file, dict, name))
        .unwrap_or(1)
        .max(1);
    content::pieces(&operations[operator.origin.operation])
        .iter()
        .map(|piece| match piece {
            content::Piece::Codes(bytes) => bytes.len() / width,
            content::Piece::Kern(_) => 0,
        })
        .sum()
}

/// What puts the fill colour back **as the stream had it** after operation `at`:
/// the very operators that set it, copied from the stream, not a stand-in made
/// from the colour PDFium reports.
///
/// A `rg` for what PDFium read is not the same state as the CMYK black the stream
/// set: everything after that draws in the inherited colour would draw in a
/// DeviceRGB stand-in for it. So the stream is read back from `at`, as far as the
/// state goes:
///
/// - the nearest `g`, `rg` or `k` — or `cs`, which also names the space — is the
///   whole of it, with the nearest `sc` or `scn` after it when there was one;
/// - a `q` … `Q` block that closed before `at` is not in force, so what was set
///   inside it is stepped over; one that is still open is, and what was set
///   before it still holds;
/// - where nothing set a colour, it is the page's own initial one, black in
///   DeviceGray, and `0 g` puts it back.
///
/// Only the fill colour: that is the one text is drawn in. Backwards from `at`
/// every time rather than carried forward through the stream: a batch hides a
/// few dozen pieces, and a walk back stops at the nearest colour.
fn fill_before(
    stream: &[u8],
    operations: &[crate::pdf::content::Operation],
    at: usize,
) -> Vec<u8> {
    let mut closed = 0usize;
    let mut value: Option<usize> = None;
    for index in (0..at).rev() {
        match operations[index].operator.as_slice() {
            b"Q" => closed += 1,
            b"q" => closed = closed.saturating_sub(1),
            b"g" | b"rg" | b"k" | b"cs" if closed == 0 => {
                let mut put = stream[operations[index].span.clone()].to_vec();
                if let Some(v) = value {
                    put.push(b'\n');
                    put.extend_from_slice(&stream[operations[v].span.clone()]);
                }
                return put;
            }
            b"sc" | b"scn" if closed == 0 && value.is_none() => value = Some(index),
            _ => {}
        }
    }
    match value {
        Some(v) => stream[operations[v].span.clone()].to_vec(),
        None => b"0 g".to_vec(),
    }
}

/// Operation `at` — a show-text operator — drawn in `colour`, with the fill
/// colour put back exactly as it was ([`fill_before`]) so nothing drawn after it
/// is touched. Colour operators are legal inside a text object, which `q` and `Q`
/// are not.
fn drawn_in_colour(
    stream: &[u8],
    operations: &[crate::pdf::content::Operation],
    at: usize,
    colour: crate::document::Color,
) -> Vec<u8> {
    let part = |v: u8| f32::from(v) / 255.0;
    let mut out =
        format!("{:.3} {:.3} {:.3} rg\n", part(colour.r), part(colour.g), part(colour.b)).into_bytes();
    out.extend_from_slice(&stream[operations[at].span.clone()]);
    out.push(b'\n');
    out.extend(fill_before(stream, operations, at));
    out
}

/// What is left of a show-text operator once it shows nothing.
///
/// `Tj` and `TJ` leave nothing: they draw and advance the pen. The quote
/// operators do more — `'` moves to the next line first, and `"` also sets the
/// word and character spacing — and the lines after one are placed from where it
/// left the line matrix. Cut out whole, it would pull every line after it up one
/// line, so what it leaves is `T*`, with the `Tw` and `Tc` a `"` set.
///
/// `Unsupported` when a `"` carries operands that are not numbers: it cannot be
/// written back, and guessing would move the page.
fn quote_effect(operation: &crate::pdf::content::Operation) -> Result<Vec<u8>> {
    match operation.operator.as_slice() {
        b"'" => Ok(b"T*".to_vec()),
        b"\"" => {
            let numbers: Option<Vec<f64>> = operation
                .operands
                .iter()
                .rev()
                .skip(1)
                .take(2)
                .map(|o| o.as_f64())
                .collect();
            let [ac, aw] = numbers.as_deref().unwrap_or_default() else {
                return Err(PdfError::Unsupported("that text is drawn by a quote operator this cannot read"));
            };
            let mut out = number_operand(*aw as f32)?;
            out.extend_from_slice(b" Tw\n");
            out.extend(number_operand(*ac as f32)?);
            out.extend_from_slice(b" Tc\nT*");
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

/// The text rendering mode in force at operation `at`: that of the nearest `Tr`
/// before it that is still in force (not inside a `q`..`Q` block that closed),
/// or `0` — fill — where none was set. Modes 4 to 7 add what is drawn to the
/// clipping path, which is why taking such text out changes more than the text.
fn render_mode_before(operations: &[crate::pdf::content::Operation], at: usize) -> i64 {
    let mut closed = 0usize;
    for index in (0..at).rev() {
        match operations[index].operator.as_slice() {
            b"Q" => closed += 1,
            b"q" => closed = closed.saturating_sub(1),
            b"Tr" if closed == 0 => {
                return operations[index].operands.last().and_then(|o| o.as_f64()).map_or(0, |v| v as i64);
            }
            _ => {}
        }
    }
    0
}

/// The show-text operators that stay and that the pen carries on to from
/// show-text operation `at` — the ones taking `at` out would move.
///
/// A piece drawn with nothing repositioning before it starts where the piece
/// before it *ended*, so cutting that one out moves it. What places a piece of
/// its own is a `Tm`, `Td`, `TD` or `T*`, a new text object (`BT`, `ET`), or a
/// `'` or `"`, which move to the next line before they draw: the walk stops at
/// the first of those. Pieces that are being taken out too (`removed`) are not
/// carried, but do not stop it either — what follows them is moved as well.
///
/// Whether moving one matters is another question: see [`draws_nothing`].
fn carried_after_removal(
    operations: &[crate::pdf::content::Operation],
    at: usize,
    removed: &std::collections::HashSet<usize>,
) -> Vec<usize> {
    let mut carried = Vec::new();
    for (index, operation) in operations.iter().enumerate().skip(at + 1) {
        match operation.operator.as_slice() {
            b"Tm" | b"Td" | b"TD" | b"T*" | b"BT" | b"ET" | b"'" | b"\"" => break,
            b"Tj" | b"TJ" if !removed.contains(&index) => carried.push(index),
            _ => {}
        }
    }
    carried
}

/// Whether a text object draws nothing — so that moving it moves no pixel.
///
/// A space is the usual case: Illustrator writes one as a `( ) Tj` of its own
/// between the pieces of a line; it advances the pen like any other piece and has
/// no ink, so PDFium reports it with no text and a box with no height — and no
/// width either when it is a zero-width one, though a justified line's space is
/// measured 16 pt wide and 0 tall. Not the same as an object with *little* area:
/// a lone `l` in a light face is 0.4 pt wide and a hyphen 0.4 pt tall, and both
/// are there to be seen. So: a side of no length (under a hundredth of a point),
/// since ink has both, or drawn invisibly (render mode 3), or fully transparent.
fn draws_nothing(run: &crate::document::TextRun, render_mode: i64) -> bool {
    render_mode == 3
        || run.color.a == 0
        || (run.rect.right - run.rect.left).abs() < 0.01
        || (run.rect.bottom - run.rect.top).abs() < 0.01
}

/// One PDF number, refusing anything that cannot be written as one.
///
/// PDF numbers have no exponent form and no infinity, so a value that needs one
/// would be a syntax error in the middle of a content stream — a corrupt page,
/// not a failed edit.
fn number_operand(v: f32) -> Result<Vec<u8>> {
    if !v.is_finite() {
        return Err(PdfError::Unsupported(
            "these words are placed by a number this cannot write",
        ));
    }
    let mut text = format!("{v:.4}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    Ok(if text == "-0" { b"0".to_vec() } else { text.into_bytes() })
}

/// A `Tm` operator carrying this matrix.
///
/// **Refuses rather than writing a number no reader can parse.** PDF numbers
/// have no exponent form and no infinity, so a matrix holding one would be a
/// syntax error in the middle of a content stream — which is a corrupt page,
/// not a failed move.
fn text_matrix(m: &[f32; 6]) -> Result<Vec<u8>> {
    matrix_operator(m, "Tm")
}

/// A `cm` operator carrying this matrix.
fn concat_matrix(m: &[f32; 6]) -> Result<Vec<u8>> {
    matrix_operator(m, "cm")
}

/// Multiply two PDF matrices, `a b c d e f`.
fn matrices(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// A box's four corners through a matrix, and the box round them again —
/// as `left, bottom, right, top`, the order PDFium reports bounds in.
fn bounds_through(l: f32, b: f32, r: f32, t: f32, m: [f32; 6]) -> (f32, f32, f32, f32) {
    if m == IDENTITY_MATRIX {
        return (l, b, r, t);
    }
    let corners = [(l, b), (r, b), (l, t), (r, t)].map(|(x, y)| {
        (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
    });
    let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        lo_x = lo_x.min(x);
        lo_y = lo_y.min(y);
        hi_x = hi_x.max(x);
        hi_y = hi_y.max(y);
    }
    (lo_x, lo_y, hi_x, hi_y)
}

/// The inverse of a PDF matrix, or `None` where there is none.
fn inverse(m: [f32; 6]) -> Option<[f32; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-9 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

fn matrix_operator(m: &[f32; 6], operator: &str) -> Result<Vec<u8>> {
    fn number(v: f32) -> Result<String> {
        if !v.is_finite() {
            return Err(PdfError::Unsupported(
                "these words are placed by a matrix this cannot write",
            ));
        }
        // Six places holds a page-space position to well under a thousandth of
        // a point, and the trim keeps the stream readable.
        let mut text = format!("{v:.6}");
        if text.contains('.') {
            text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        Ok(if text == "-0" { "0".into() } else { text })
    }

    let parts: Result<Vec<String>> = m.iter().map(|v| number(*v)).collect();
    Ok(format!("{} {operator}", parts?.join(" ")).into_bytes())
}

/// A show-text operand as a hex string — safe for any byte value, unlike a
/// literal string, which would need its parentheses and backslashes escaped
/// and a code that happens to contain one is not this function's problem to
/// notice.
fn hex_string_operand(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2 + 2);
    out.push(b'<');
    for b in bytes {
        out.extend_from_slice(format!("{b:02X}").as_bytes());
    }
    out.push(b'>');
    out
}

/// The graphics and text state set before an operation, as the bytes that set
/// it — colour, line width, the `gs` that carries transparency, the font.
///
/// **Copied rather than parsed.** Re-serialising a colour means knowing its
/// space, and a `/GS0 gs` means nothing without the dictionary it names. The
/// operators that set the state are already in the stream and already correct,
/// so they are lifted whole and put back in the order they were last set —
/// which keeps `cs` in front of the `scn` that needs it.
fn state_in_force(operations: &[crate::pdf::content::Operation], stream: &[u8], upto: usize) -> Vec<Vec<u8>> {
    const TRACKED: &[&[u8]] = &[
        b"g", b"G", b"rg", b"RG", b"k", b"K", b"cs", b"CS", b"sc", b"scn", b"SC", b"SCN", b"gs",
        b"w", b"J", b"j", b"M", b"d", b"ri", b"i", b"Tf", b"Tc", b"Tw", b"Tz", b"TL", b"Ts", b"Tr",
    ];

    let mut live: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut stack: Vec<Vec<(Vec<u8>, Vec<u8>)>> = Vec::new();
    for operation in operations.iter().take(upto) {
        match operation.operator.as_slice() {
            b"q" => stack.push(live.clone()),
            b"Q" => {
                if let Some(saved) = stack.pop() {
                    live = saved;
                }
            }
            other if TRACKED.contains(&other) => {
                live.retain(|(name, _)| name != other);
                live.push((
                    other.to_vec(),
                    stream.get(operation.span.clone()).unwrap_or_default().to_vec(),
                ));
            }
            _ => {}
        }
    }
    live.into_iter().map(|(_, bytes)| bytes).collect()
}

/// One clipping path in force at some point in a stream: the operations that
/// build it, whether it used the even-odd rule, and the transform it was set
/// under.
#[derive(Debug, Clone)]
struct ClipInForce {
    /// The construction operators, `m`/`re`/`l`/… up to but not including the
    /// `W` — so they can be replayed anywhere.
    construction: std::ops::Range<usize>,
    even_odd: bool,
    ctm: [f32; 6],
}

/// Every clipping path in force where an operation sits, outermost first.
///
/// **What lets something drawn inside a clip be re-stacked.** A clip is set by
/// `W` and lasts until the enclosing `Q`, so something lifted out from under
/// one and put back at the other end of the stream arrives unclipped, and
/// paints over whatever the clip was keeping it off. The first version refused
/// that case outright — measured on a real brochure page, that was 22 of 50
/// shapes and 9 of the words and pictures, which is most of what anyone would
/// want to move. Carrying the clips is the answer: each one is replayed, under
/// the transform it was set under, inside the block that carries the object.
fn clips_in_force(
    operations: &[crate::pdf::content::Operation],
    states: &[crate::pdf::content::State],
    upto: usize,
) -> Vec<ClipInForce> {
    // Per `q` level, the clips set at that level.
    let mut levels: Vec<Vec<ClipInForce>> = vec![Vec::new()];
    let mut path_start: Option<usize> = None;
    let mut pending: Option<bool> = None;
    for (index, operation) in operations.iter().enumerate().take(upto) {
        match operation.operator.as_slice() {
            b"q" => levels.push(Vec::new()),
            b"Q" => {
                if levels.len() > 1 {
                    levels.pop();
                }
            }
            b"m" | b"re" => {
                if path_start.is_none() {
                    path_start = Some(index);
                }
            }
            b"l" | b"c" | b"v" | b"y" | b"h" => {}
            b"W" => pending = Some(false),
            b"W*" => pending = Some(true),
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                if let (Some(from), Some(even_odd)) = (path_start, pending) {
                    // The clip takes effect after the painting operator, under
                    // the transform in force for the path — which is the one at
                    // its first construction operator.
                    let ctm = states.get(from).map(|s| s.ctm).unwrap_or(IDENTITY_MATRIX);
                    if let Some(level) = levels.last_mut() {
                        level.push(ClipInForce { construction: from..index, even_odd, ctm });
                    }
                }
                path_start = None;
                pending = None;
            }
            _ => {}
        }
    }
    levels.into_iter().flatten().collect()
}

/// The nearest point after an operation where no clip is in force: past the
/// `Q` that closes each clipping scope, innermost first.
fn leave_clips_after(
    operations: &[crate::pdf::content::Operation],
    states: &[crate::pdf::content::State],
    mut at: usize,
) -> usize {
    loop {
        if clips_in_force(operations, states, at + 1).is_empty() {
            return at;
        }
        // The `Q` that closes the level `at` is in: the first one reached at
        // depth zero, counting the `q`s opened on the way.
        let mut depth = 0usize;
        let mut index = at + 1;
        let closing = loop {
            let Some(operation) = operations.get(index) else { break None };
            match operation.operator.as_slice() {
                b"q" => depth += 1,
                b"Q" => {
                    if depth == 0 {
                        break Some(index);
                    }
                    depth -= 1;
                }
                _ => {}
            }
            index += 1;
        };
        // Unbalanced: land where asked rather than nowhere.
        let Some(closing) = closing else { return at };
        at = closing;
    }
}

/// The nearest point before an operation where no clip is in force: before
/// the `q` that opens each clipping scope, innermost first.
fn leave_clips_before(
    operations: &[crate::pdf::content::Operation],
    states: &[crate::pdf::content::State],
    mut at: usize,
) -> usize {
    loop {
        if clips_in_force(operations, states, at).is_empty() {
            return at;
        }
        let mut depth = 0usize;
        let mut index = at;
        let opening = loop {
            if index == 0 {
                break None;
            }
            index -= 1;
            match operations[index].operator.as_slice() {
                b"Q" => depth += 1,
                b"q" => {
                    if depth == 0 {
                        break Some(index);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        };
        let Some(opening) = opening else { return at };
        at = opening;
    }
}

/// Whether an operation sits inside a `BT … ET` text object.
fn inside_text_object(operations: &[crate::pdf::content::Operation], at: usize) -> bool {
    let mut open = false;
    for operation in operations.iter().take(at + 1) {
        match operation.operator.as_slice() {
            b"BT" => open = true,
            b"ET" => open = false,
            _ => {}
        }
    }
    open
}

const IDENTITY_MATRIX: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn mark(area: Rect, page_height: f32, fill: crate::document::Color) -> Vec<u8> {
    // The area arrives top-left down; a content stream works bottom-left up.
    let (bottom, top) = (page_height - area.bottom, page_height - area.top);
    let (r, g, b) = (
        f32::from(fill.r) / 255.0,
        f32::from(fill.g) / 255.0,
        f32::from(fill.b) / 255.0,
    );
    format!(
        "\nq\n{r} {g} {b} rg\n{} {} {} {} re\nf\nQ\n",
        area.left,
        bottom,
        area.right - area.left,
        top - bottom
    )
    .into_bytes()
}

/// An image object that draws nothing at all.
///
/// **An image mask, not a blank picture.** The object keeps the matrix that
/// placed the original, so anything it draws is stretched across the whole
/// frame — a one-pixel white image would paint a white rectangle over whatever
/// sits behind it, and a black one a black rectangle. A mask whose only sample
/// is 1 leaves the page exactly as it was, which is what "the picture is gone"
/// has to look like.
///
/// Built fresh rather than by editing the original dictionary: that carried a
/// `/Filter`, a `/ColorSpace`, very likely an `/SMask`, and each of them would
/// contradict a one-bit mask or point back at data that is no longer there.
fn blank_image_object() -> Vec<u8> {
    use crate::pdf::Object;
    let dict = crate::pdf::Dict(vec![
        (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Image".to_vec())),
        (b"Width".to_vec(), Object::Number(b"1".to_vec())),
        (b"Height".to_vec(), Object::Number(b"1".to_vec())),
        (b"BitsPerComponent".to_vec(), Object::Number(b"1".to_vec())),
        (b"ImageMask".to_vec(), Object::Bool(true)),
    ]);
    // Every bit set: with the default `/Decode`, a 1 in a mask leaves the page
    // unchanged.
    crate::pdf::write_stream(&dict, &[0xFF])
}

/// How many entries are in each page's own `/Annots` array, as the file
/// itself says — 0 where a page has none. **Not inherited**: unlike
/// `/Resources` or `/MediaBox`, `/Annots` is a page's own; there is nothing
/// to walk up to. One pass over the page tree for every page's count, rather
/// than one walk of it per page — the difference between this and quadratic
/// on the hundred-and-forty-nine-page catalogue this engine is measured
/// against elsewhere.
fn annots_counts(file: &crate::pdf::File<'_>) -> Result<Vec<usize>> {
    let root = file.resolve(
        file.trailer()
            .get(b"Root")
            .ok_or_else(|| PdfError::InvalidArgument("the file has no catalogue".into()))?,
    )?;
    let pages = file.resolve(
        root.as_dict()
            .and_then(|d| d.get(b"Pages"))
            .ok_or_else(|| PdfError::InvalidArgument("the file has no page tree".into()))?,
    )?;
    let mut flat = Vec::new();
    collect_pages(file, &pages, &mut flat, &mut HashSet::new(), 0)?;
    Ok(flat
        .iter()
        .map(|page| {
            match page.as_dict().and_then(|d| d.get(b"Annots")).map(|a| file.resolve(a)) {
                Some(Ok(crate::pdf::Object::Array(items))) => items.len(),
                _ => 0,
            }
        })
        .collect())
}

/// Every page in a page tree, in order.
///
/// Walked rather than indexed: `/Kids` may nest, and a tree of a hundred and
/// forty-nine pages is rarely flat.
fn collect_pages(
    file: &crate::pdf::File<'_>,
    node: &crate::pdf::Object,
    out: &mut Vec<crate::pdf::Object>,
    seen: &mut HashSet<u32>,
    depth: usize,
) -> Result<()> {
    use crate::pdf::Object;

    // A `/Kids` that points back at an ancestor would otherwise walk for ever.
    if depth > 64 {
        return Err(PdfError::InvalidArgument("the page tree nests too deeply".into()));
    }
    let Some(dict) = node.as_dict() else { return Ok(()) };

    if dict.get(b"Type").and_then(Object::as_name) == Some(&b"Page"[..]) {
        out.push(node.clone());
        return Ok(());
    }
    let Some(kids) = dict.get(b"Kids") else { return Ok(()) };
    if let Object::Array(items) = file.resolve(kids)? {
        for kid in items {
            // A page tree that names the same child at every level is a DAG,
            // not a tree — visited by reference, not by depth alone, or the
            // same double-per-level blowup the depth cap does not stop.
            // Found by audit.
            if let Object::Reference(number, _) = &kid {
                if !seen.insert(*number) {
                    continue;
                }
            }
            let kid = file.resolve(&kid)?;
            collect_pages(file, &kid, out, seen, depth + 1)?;
        }
    }
    Ok(())
}

/// An attachment's name, or an empty string if it has none that can be read.
fn attachment_name(
    bindings: &dyn PdfiumLibraryBindings,
    attachment: pdfium_render::prelude::FPDF_ATTACHMENT,
) -> String {
    let wanted = unsafe { bindings.FPDFAttachment_GetName(attachment, std::ptr::null_mut(), 0) };
    if wanted == 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; (wanted as usize / 2).max(1)];
    let written = unsafe {
        bindings.FPDFAttachment_GetName(attachment, buffer.as_mut_ptr(), wanted)
    };
    let len = (written as usize / 2).saturating_sub(1).min(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Whether a redaction is being measured or carried out.
///
/// The survey runs identically either way — the same walk, the same judgements,
/// the same report — and one of them stops before touching anything. Sharing the
/// code rather than writing a second, read-only version is the point: a preview
/// that disagreed with the redaction it previews would be worse than none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    /// Look, report, change nothing.
    Survey,
    Apply,
}

impl PdfiumDocument {
    /// Destroy every piece of content inside a rectangle.
    ///
    /// **Surveyed whole, then applied.** Nothing is removed until the entire
    /// page has been judged, so a rectangle that turns out to cover something
    /// this pass cannot clear leaves the page exactly as it was. Half a
    /// redaction is worse than none: the file looks treated and is not.
    fn redact_inner(
        &mut self,
        request: &Redaction,
        intent: Intent,
        catalogue: Option<&crate::document::glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        use crate::document::redact::{
            contains, covered_share, fate, overlaps, slice, Fate, Marked, Segment,
        };

        self.validate_page_index(request.page_index)?;

        // A page whose words are not text objects cannot be redacted by
        // removing text objects. Refused by name — the alternative is removing
        // nothing, reporting success, and painting a mark over words that are
        // still there. Not overridable by `require_complete`, because there is
        // nothing to override *to*: forcing it would produce exactly that lie.
        //
        // **`Outlined` is the one exception, and only when a `catalogue` was
        // given.** Without a font to match against, this refusal is exactly
        // what it always was. With one, refusing outright is now the wrong
        // answer in the ordinary case — a rectangle drawn around a whole word
        // already removes every path wholly inside it, curves or not, with no
        // matching needed at all. What a catalogue adds beyond that is
        // explained where `removable_outlined` is built, a few lines down.
        let classification = self.page(request.page_index)?.classify()?;
        // A page holding real text *and* an image over most of it is a scan with
        // a caption. Any image on it may carry words the caption does not.
        let scanned_content =
            classification.image_coverage >= PageClassification::SCAN_COVERAGE;
        match classification.kind {
            PageTextKind::Outlined if catalogue.is_none() => {
                return Err(PdfError::Unsupported(
                    "this page's text is drawn as curves, which redaction cannot remove \
                     without a font to match it against",
                ))
            }
            PageTextKind::Scanned => {
                return Err(PdfError::Unsupported(
                    "this page is a scan, so its words are part of the image, not text",
                ))
            }
            _ => {}
        }

        // **A second, independent page handle — read, then dropped, before the
        // one below is opened.** Identification only reads; nothing here is
        // mutated until the `RawPage` further down exists and the object
        // indices the two agree on are exactly what makes that safe. Both walk
        // the same, unmodified `FPDFPage_GetObject(page, i)` list in the same
        // order, and nothing between the two opens changes the page's content
        // to make that list disagree with itself.
        //
        // The alternative — reaching for `self.page.objects()` on the *same*
        // handle `RawPage` is about to mutate — is the hazard
        // `set_text_run_styled` already paid for once: two live handles on one
        // page, one of them about to rewrite its content stream, is how a page
        // ends up with something drawn twice under one handle's view and
        // gone under the other's.
        let removable_outlined: HashSet<usize> = match catalogue {
            Some(catalogue) if classification.kind == PageTextKind::Outlined => {
                // The concrete `PdfPage`, opened the same way `Document::page`
                // does — not through that method itself, which boxes it behind
                // `dyn Page` and erases exactly the type `identify_outlined_glyphs`
                // needs. Dropped at the end of this block, well before `raw`
                // below is opened.
                let pdfium_index = i32::try_from(request.page_index).map_err(|_| {
                    PdfError::InvalidArgument(format!(
                        "page index {} is out of range",
                        request.page_index
                    ))
                })?;
                let page = self
                    .document
                    .pages()
                    .get(pdfium_index)
                    .map_err(|e| PdfError::Pdfium(e.to_string()))?;
                identify_outlined_glyphs(&page, catalogue)
                    .into_iter()
                    .filter(|i| overlaps(&i.bounds.into(), &request.area))
                    .flat_map(|i| i.objects)
                    .collect()
            }
            _ => HashSet::new(),
        };

        let page_number = i32::try_from(request.page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {} is out of range", request.page_index))
        })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let space = raw.space()?;
        let bindings = pdfium()?.bindings();
        let page_width = unsafe { bindings.FPDF_GetPageWidthF(raw.handle) };
        let page_height = unsafe { bindings.FPDF_GetPageHeightF(raw.handle) };
        let area = request.area;

        // ------------------------------------------------------------ survey --

        // Handles, and kept: `FPDFPage_RemoveObject` takes a handle, and an
        // index would be wrong the moment the first removal renumbered
        // everything after it.
        //
        // **Form XObjects are descended into rather than refused over.** The
        // first version reported any form whose bounding box crossed the
        // rectangle, and measuring it on a real catalogue showed what that
        // costs: 299 forms crossing a mid-page rectangle over 149 pages, and
        // every page refused. Nearly all of them hold nothing in the area at
        // all. Walking their children answers the question the bounding box
        // only guessed at.
        //
        // Their children are surveyed and **never removed**, which is the other
        // half of the same fact: an XObject exists to be drawn more than once,
        // so `FPDFFormObj_RemoveObject` on a shared form would blank content on
        // pages nobody was redacting. Anything of theirs inside the rectangle is
        // reported instead, and refuses.
        let mut objects: Vec<FPDF_PAGEOBJECT> = Vec::new();
        let mut inside_form: Vec<bool> = Vec::new();
        // Which form each nested object is drawn through — the one whose
        // stream holds its operators.
        let mut parent_of: Vec<Option<usize>> = Vec::new();
        // Every position a handle holds. **A form drawn twice has one set of
        // children**: PDFium parses the stream once and both drawings list
        // the same objects, so a nested handle can stand for two drawings —
        // one position each — and its characters come from the text page in
        // drawing order, the first drawing's, then the second's.
        let mut positions_of: HashMap<usize, Vec<usize>> = HashMap::new();

        // What takes each object's own space to the page's: identity for the
        // page's own, the form objects' matrices innermost first for one
        // inside a form — PDFium reports a nested object's bounds in the
        // form's space, not the page's.
        let mut to_page_of: Vec<[f32; 6]> = Vec::new();

        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
        let mut queue: Vec<(FPDF_PAGEOBJECT, bool, Option<usize>, [f32; 6])> = Vec::new();
        for i in 0..count {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, i) };
            if !handle.is_null() {
                queue.push((handle, false, None, IDENTITY_MATRIX));
            }
        }

        // Depth-limited: a form may legally reference another, and a damaged
        // file may do so in a circle.
        let mut depth = 0;
        while !queue.is_empty() && depth < 8 {
            let mut next = Vec::new();
            for (handle, nested, parent, to_page) in queue.drain(..) {
                let position = objects.len();
                positions_of.entry(handle as usize).or_default().push(position);
                objects.push(handle);
                inside_form.push(nested);
                parent_of.push(parent);
                to_page_of.push(to_page);

                if unsafe { bindings.FPDFPageObj_GetType(handle) } as u32 == FPDF_PAGEOBJ_FORM {
                    let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
                    unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
                    let inner = matrices([m.a, m.b, m.c, m.d, m.e, m.f], to_page);
                    let children = unsafe { bindings.FPDFFormObj_CountObjects(handle) };
                    for i in 0..children {
                        let child = unsafe { bindings.FPDFFormObj_GetObject(handle, i as c_ulong) };
                        if !child.is_null() {
                            next.push((child, true, Some(position), inner));
                        }
                    }
                }
            }
            queue = next;
            depth += 1;
        }

        // **Where every form is, as a path of ordinals from the page.** The
        // page's own forms are drawn in the order their `Do` operators come
        // in its stream, so the n-th form PDFium lists is the n-th form `Do`
        // there; a form inside one is the m-th form `Do` of *that* form's
        // stream, among its children in order. `[n, m]` is how a form here
        // is found in the file there, however deep.
        let mut form_paths: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut siblings_seen: HashMap<Option<usize>, usize> = HashMap::new();
        for (position, handle) in objects.iter().enumerate() {
            if unsafe { bindings.FPDFPageObj_GetType(*handle) } as u32 != FPDF_PAGEOBJ_FORM {
                continue;
            }
            let parent = parent_of[position];
            let ordinal = siblings_seen.entry(parent).or_insert(0);
            let mut path = parent.and_then(|p| form_paths.get(&p).cloned()).unwrap_or_default();
            if parent.is_some() && path.is_empty() {
                // A parent the walk did not place: neither is this.
                *ordinal += 1;
                continue;
            }
            path.push(*ordinal);
            *ordinal += 1;
            form_paths.insert(position, path);
        }

        let mut marks: HashMap<usize, Vec<Marked>> = HashMap::new();
        // Each character's box, page space, in the order the marks come — what
        // slicing a run drawn through a form needs, since its operators are in
        // a stream the page's glyph list cannot be asked about.
        let mut boxes: HashMap<usize, Vec<Rect>> = HashMap::new();
        let mut loose_boxes: HashMap<usize, Vec<Rect>> = HashMap::new();
        let mut texts: HashMap<usize, String> = HashMap::new();
        let mut unreadable: HashSet<usize> = HashSet::new();
        let mut report = RedactionReport::default();
        let mut nested_text = false;

        let text_page = unsafe { bindings.FPDFText_LoadPage(raw.handle) };
        // For an object several drawings share: how many characters the
        // text page gives it altogether — its own, once per drawing — and
        // how many have been handed out so far.
        let mut chars_of: HashMap<usize, usize> = HashMap::new();
        let mut seen_of: HashMap<usize, usize> = HashMap::new();
        // Each nested object's words as the text page spells them, one
        // character per box, so the two cannot disagree about their count.
        let mut walked: HashMap<usize, String> = HashMap::new();
        if !text_page.is_null() {
            let chars = unsafe { bindings.FPDFText_CountChars(text_page) };
            for index in 0..chars {
                if unsafe { bindings.FPDFText_IsGenerated(text_page, index) } == 1 {
                    continue;
                }
                let owner = unsafe { bindings.FPDFText_GetTextObject(text_page, index) };
                if positions_of.get(&(owner as usize)).is_some_and(|p| p.len() > 1) {
                    *chars_of.entry(owner as usize).or_insert(0) += 1;
                }
            }
            for index in 0..chars {
                // Characters PDFium invented — the spaces it inserts between
                // runs so extracted text reads as words — are not in the
                // object's own string. Counting them shifts every offset after
                // the first and splits the run in the wrong place.
                if unsafe { bindings.FPDFText_IsGenerated(text_page, index) } == 1 {
                    continue;
                }

                let owner = unsafe { bindings.FPDFText_GetTextObject(text_page, index) };
                if owner.is_null() {
                    continue;
                }
                let Some(positions) = positions_of.get(&(owner as usize)) else {
                    // A character whose object is not in the page's own list
                    // lives deeper than the walk went, and cannot be addressed
                    // for removal from here.
                    nested_text = true;
                    continue;
                };
                let object = if positions.len() == 1 {
                    positions[0]
                } else {
                    // The same object drawn through several drawings of one
                    // form: its characters arrive one drawing at a time, so
                    // the n-th run of them belongs to the n-th drawing.
                    let per = (chars_of.get(&(owner as usize)).copied().unwrap_or(0) / positions.len()).max(1);
                    let seen = seen_of.entry(owner as usize).or_insert(0);
                    let drawing = *seen / per;
                    *seen += 1;
                    match positions.get(drawing) {
                        Some(position) => *position,
                        None => {
                            nested_text = true;
                            continue;
                        }
                    }
                };

                let (mut l, mut r, mut b, mut t) = (0f64, 0f64, 0f64, 0f64);
                let read = unsafe {
                    bindings.FPDFText_GetCharBox(text_page, index, &mut l, &mut r, &mut b, &mut t)
                } != 0;
                if !read {
                    // Where it sits is unknown, so whether the rectangle covers
                    // it is unknowable. Flagged so the whole run goes rather
                    // than guessing character by character.
                    unreadable.insert(object);
                    marks.entry(object).or_default().push(Marked { covered: true, left: 0.0 });
                    boxes.entry(object).or_default().push(Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 });
                    if inside_form[object] {
                        walked.entry(object).or_default().push('\u{FFFD}');
                        loose_boxes.entry(object).or_default().push(Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 });
                    }
                    continue;
                }

                let (left, top) = space.to_top_left(l as f32, t as f32);
                let (right, bottom) = space.to_top_left(r as f32, b as f32);
                let glyph = Rect { left, top, right, bottom };
                marks
                    .entry(object)
                    .or_default()
                    .push(Marked { covered: overlaps(&glyph, &area), left: l as f32 });
                boxes.entry(object).or_default().push(glyph);
                if inside_form[object] {
                    let mut wide = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
                    let loose = if unsafe { bindings.FPDFText_GetLooseCharBox(text_page, index, &mut wide) } != 0 {
                        let (l, t) = space.to_top_left(wide.left, wide.top);
                        let (r, b) = space.to_top_left(wide.right, wide.bottom);
                        Rect { left: l.min(r), top: t.min(b), right: l.max(r), bottom: t.max(b) }
                    } else {
                        glyph
                    };
                    loose_boxes.entry(object).or_default().push(loose);
                    let unicode = unsafe { bindings.FPDFText_GetUnicode(text_page, index) };
                    walked
                        .entry(object)
                        .or_default()
                        .push(char::from_u32(unicode).unwrap_or('\u{FFFD}'));
                }
            }

            // Read while the text page is open: `FPDFTextObj_GetText` needs one,
            // and opening a second later — with this page's content about to be
            // regenerated — is how a page ends up with its text drawn twice.
            for (position, &handle) in objects.iter().enumerate() {
                if marks.contains_key(&position) {
                    texts.insert(position, object_text(bindings, text_page, handle));
                }
            }
            unsafe { bindings.FPDFText_ClosePage(text_page) };
        }

        if nested_text {
            report.uncleared.push(Uncleared::Form { object: usize::MAX });
        }

        // What happens to each object, decided before anything is touched.
        enum Act {
            Remove,
            Rewrite(Vec<Segment>),
        }
        let mut plan: Vec<(usize, Act)> = Vec::new();
        // Words drawn through a form, to be cut out of the form's own stream
        // — see `plan_form_cuts`.
        let mut nested_covered: Vec<NestedRun> = Vec::new();

        for (position, &handle) in objects.iter().enumerate() {
            let kind = unsafe { bindings.FPDFPageObj_GetType(handle) } as u32;

            if kind == FPDF_PAGEOBJ_TEXT {
                let Some(marked) = marks.get(&position) else { continue };
                let text = texts.get(&position).cloned().unwrap_or_default();

                // Inside a form: not one of the page's own objects, so not
                // removable here — gathered for the pass over the form's
                // stream instead, which is where its operators are.
                if inside_form[position] {
                    if marked.iter().any(|m| m.covered) {
                        let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
                        unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
                        nested_covered.push(NestedRun {
                            position,
                            parent: parent_of[position],
                            // As the text page walked it, not as the object
                            // reports itself: an object several drawings
                            // share reports the generated space between
                            // them as its own.
                            text: walked.get(&position).cloned().unwrap_or_else(|| text.clone()),
                            origin: (m.e, m.f),
                            boxes: boxes.get(&position).cloned().unwrap_or_default(),
                            loose: loose_boxes.get(&position).cloned().unwrap_or_default(),
                            unreadable: unreadable.contains(&position),
                        });
                    }
                    continue;
                }

                // **The alignment guard.** Splitting indexes the object's own
                // string with offsets counted during a walk of the page. If the
                // two disagree — a synthesised character missed, an object whose
                // characters were not visited together — those offsets address
                // the wrong letters, and removing the wrong letters is worse
                // than removing too many. A mismatch takes the whole run and
                // says so in the report.
                let aligned = marked.len() == text.chars().count();

                match fate(marked) {
                    Fate::Untouched => {}
                    Fate::Whole => {
                        report.characters += marked.len();
                        report.objects += 1;
                        plan.push((position, Act::Remove));
                    }
                    Fate::Split(segments) => {
                        let rotated = {
                            let mut m =
                                FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
                            unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
                            m.b.abs() > 1e-4 || m.c.abs() > 1e-4
                        };

                        report.characters += marked.iter().filter(|m| m.covered).count();

                        if !aligned || rotated || unreadable.contains(&position) {
                            // Over-redact rather than mis-redact. Rebuilding a
                            // rotated run means placing each surviving piece
                            // along a rotated baseline, and an axis-aligned
                            // bounding box does not say where that is.
                            report.objects += 1;
                            for segment in &segments {
                                let spilled = slice(&text, segment);
                                if !spilled.is_empty() {
                                    report.spilled.push(spilled);
                                }
                            }
                            plan.push((position, Act::Remove));
                        } else {
                            plan.push((position, Act::Rewrite(segments)));
                        }
                    }
                }
                continue;
            }

            let (mut l, mut b, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
            if unsafe { bindings.FPDFPageObj_GetBounds(handle, &mut l, &mut b, &mut r, &mut t) } == 0
            {
                continue;
            }
            // On the page, wherever the object's own space is.
            let (l, b, r, t) = bounds_through(l, b, r, t, to_page_of[position]);
            let (left, top) = space.to_top_left(l, t);
            let (right, bottom) = space.to_top_left(r, b);
            let bounds = Rect { left, top, right, bottom };

            // A form is a container. Its children were queued and are judged on
            // their own, so the box it happens to occupy says nothing.
            if kind == FPDF_PAGEOBJ_FORM {
                continue;
            }

            if inside_form[position] {
                // Judged by exactly the rule its top-level equivalent gets. The
                // first version called every nested object a blocker, which is
                // the container-bounding-box mistake moved one level down: a
                // table rule inside a form is still a table rule.
                if !overlaps(&bounds, &area) {
                    continue;
                }
                report.uncleared.push(match kind {
                    FPDF_PAGEOBJ_IMAGE => Uncleared::Image {
                        object: position,
                        covers: covered_share(&area, &bounds),
                        may_hold_text: scanned_content,
                    },
                    FPDF_PAGEOBJ_PATH => {
                        let segments =
                            unsafe { bindings.FPDFPath_CountSegments(handle) }.max(0) as usize;
                        if crate::document::classify::looks_like_type(
                            (r - l).abs(),
                            (t - b).abs(),
                            segments,
                            page_width,
                            page_height,
                        ) {
                            Uncleared::OutlinedText { object: position }
                        } else {
                            Uncleared::Path { object: position }
                        }
                    }
                    _ => Uncleared::Form { object: position },
                });
                continue;
            }

            if contains(&area, &bounds) {
                // Wholly inside, so removing it destroys nothing outside the
                // rectangle — whatever it is. Already true for an outlined
                // letter, with or without a `catalogue`: nothing here needed
                // matching to know a fully-enclosed path is safe to take.
                report.objects += 1;
                plan.push((position, Act::Remove));
                continue;
            }
            if !overlaps(&bounds, &area) {
                continue;
            }

            // **What a `catalogue` actually buys**, beyond the wholly-inside
            // case above: a path that only *straddles* the rectangle's edge,
            // which `contains` alone can never call safe. If matching
            // confidently identified it as a letter — `removable_outlined` was
            // built from exactly the same rectangle — the whole path goes.
            // There is no partial-letter removal to fall back to the way a
            // native text run splits at a character boundary; a Bézier
            // contour has no such boundary to split at, so a straddling path
            // this pass cannot identify stays a blocker, exactly as before a
            // `catalogue` existed at all.
            if kind == FPDF_PAGEOBJ_PATH && removable_outlined.contains(&position) {
                report.objects += 1;
                plan.push((position, Act::Remove));
                continue;
            }

            report.uncleared.push(match kind {
                FPDF_PAGEOBJ_IMAGE => Uncleared::Image {
                    object: position,
                    covers: covered_share(&area, &bounds),
                    may_hold_text: scanned_content,
                },
                FPDF_PAGEOBJ_FORM => Uncleared::Form { object: position },
                FPDF_PAGEOBJ_PATH => {
                    let segments =
                        unsafe { bindings.FPDFPath_CountSegments(handle) }.max(0) as usize;
                    if crate::document::classify::looks_like_type(
                        (r - l).abs(),
                        (t - b).abs(),
                        segments,
                        page_width,
                        page_height,
                    ) {
                        Uncleared::OutlinedText { object: position }
                    } else {
                        Uncleared::Path { object: position }
                    }
                }
                _ => continue,
            });
        }

        // **Words drawn through forms, cut from the forms' own streams.** The
        // page's object list cannot remove them — an XObject exists to be drawn
        // more than once, and `FPDFFormObj_RemoveObject` on a shared one would
        // blank content on pages nobody was redacting — so they are planned
        // here and cut at the byte level after the page's own objects are
        // done: in place when the form is this page's alone, from a private
        // copy when the file draws it elsewhere too. What the plan cannot place
        // is reported as it always was.
        let form_plan = if nested_covered.is_empty() {
            FormPlan::default()
        } else {
            let bytes = self.readable_bytes()?;
            self.plan_form_cuts(&bytes, request, &nested_covered, &form_paths)?
        };
        for position in &form_plan.refused {
            report.uncleared.push(Uncleared::Form { object: *position });
        }
        for cut in &form_plan.cuts {
            report.characters += cut.characters;
            report.spilled.extend(cut.spilled.iter().cloned());
            if cut.elsewhere > 0 {
                report.uncleared.push(Uncleared::SharedForm {
                    object: cut.form,
                    elsewhere: cut.elsewhere,
                });
            }
        }

        // Annotations are overlay content: one wholly inside goes, and one
        // crossing the edge is reported rather than removed, since most of what
        // it covers is outside what was asked to be cleared.
        let mut doomed: Vec<c_int> = Vec::new();
        for index in 0..unsafe { bindings.FPDFPage_GetAnnotCount(raw.handle) } {
            let annotation = unsafe { bindings.FPDFPage_GetAnnot(raw.handle, index) };
            if annotation.is_null() {
                continue;
            }
            let mut rect = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
            let read = unsafe { bindings.FPDFAnnot_GetRect(annotation, &mut rect) } != 0;
            unsafe { bindings.FPDFPage_CloseAnnot(annotation) };
            if !read {
                continue;
            }
            let (left, top) = space.to_top_left(rect.left, rect.top);
            let (right, bottom) = space.to_top_left(rect.right, rect.bottom);
            let bounds = Rect {
                left: left.min(right),
                top: top.min(bottom),
                right: left.max(right),
                bottom: top.max(bottom),
            };
            if contains(&area, &bounds) {
                doomed.push(index);
            } else if overlaps(&bounds, &area) {
                report.uncleared.push(Uncleared::Annotation { index: index as usize });
            }
        }

        // -------------------------------------------------------------- gate --

        // A survey stops here and hands back what it found. **That is the whole
        // reason it exists.** Measuring a real catalogue showed that of the
        // pages an image refuses, three quarters had their text come out
        // perfectly cleanly and were refused over a photograph beside it.
        // Whether that photograph holds anything worth hiding is not a question
        // this code can answer, and is an easy one for the person looking at the
        // page — so the engine's job is to let them be asked, not to decide for
        // them.
        if intent == Intent::Survey {
            return Ok(report);
        }

        // **Not overridable, and deliberately checked after the survey rather
        // than before it.** Acknowledging an image is the user answering for
        // whether the picture beside their text matters. It is not, and must not
        // become, permission to paint a black rectangle over words that were
        // never removed — which is what going ahead here would produce. The
        // whole-page equivalent is the `Scanned` refusal above; this is the same
        // situation confined to one region, and it earns the same answer.
        if report.would_only_draw_a_mark() {
            // Named for what is actually there. "Part of an image" was the only
            // wording, and it was wrong for the form and outlined cases —
            // which now refuse too. Found by audit.
            return Err(PdfError::IncompleteRedaction(format!(
                "a redaction here would draw a mark and remove nothing: {}",
                report.what_survives_a_mark().join("; ")
            )));
        }

        if request.require_complete {
            let mut how: Vec<String> =
                report.blockers().iter().map(|b| b.describe()).collect();
            if !how.is_empty() {
                how.sort();
                how.dedup();
                // Returned before a single object has been removed, which is the
                // point of surveying first.
                return Err(PdfError::IncompleteRedaction(how.join("; ")));
            }
        }

        // ------------------------------------------------------------- apply --

        // **Set before the first destructive call, not after the last one.**
        // Removal and rewrite happen through PDFium's live object model one
        // object at a time, and nothing here can undo an earlier one if a
        // later one — or `FPDFPage_GenerateContent`, or the form-cut pass
        // below — fails partway through. A failure at that point must still
        // force a full copy on the next save: an incremental one would leave
        // the pre-redaction revision, words included, sitting in the file
        // right behind the one that is supposed to have removed them. Wrong
        // in the safe direction on a redaction that fails outright and never
        // touched a single object — an unneeded full copy — rather than the
        // other one. Found by audit.
        self.redacted = true;

        for (position, act) in &plan {
            let handle = objects[*position];
            match act {
                Act::Remove => {
                    if unsafe { bindings.FPDFPage_RemoveObject(raw.handle, handle) } != 0 {
                        unsafe { bindings.FPDFPageObj_Destroy(handle) };
                    }
                }
                Act::Rewrite(segments) => {
                    let text = texts.get(position).cloned().unwrap_or_default();
                    rewrite_run(
                        bindings,
                        self.document.handle(),
                        raw.handle,
                        handle,
                        &text,
                        segments,
                    )?;
                }
            }
        }

        // Backwards: removing an annotation renumbers the ones after it.
        for index in doomed.iter().rev() {
            unsafe { bindings.FPDFPage_RemoveAnnot(raw.handle, *index) };
        }

        if let Some(fill) = request.fill {
            let mark = to_pdf_rect(&space, &area);
            let rect = unsafe {
                bindings.FPDFPageObj_CreateNewRect(
                    mark.left,
                    mark.bottom,
                    mark.right - mark.left,
                    mark.top - mark.bottom,
                )
            };
            if !rect.is_null() {
                unsafe {
                    bindings.FPDFPageObj_SetFillColor(
                        rect,
                        fill.r as c_uint,
                        fill.g as c_uint,
                        fill.b as c_uint,
                        fill.a as c_uint,
                    );
                    // Filled, not stroked: an outline round the area is a
                    // border, and a redaction mark has to be solid.
                    bindings.FPDFPath_SetDrawMode(rect, 1, 0);
                    bindings.FPDFPage_InsertObject(raw.handle, rect);
                }
            }
        }

        // Without this the removals live in PDFium's object model and never
        // reach the content stream, so they survive until the save and vanish.
        if unsafe { bindings.FPDFPage_GenerateContent(raw.handle) } == 0 {
            return Err(PdfError::Pdfium("the page could not be rewritten".into()));
        }

        // The forms, with the page handle closed first: the cut rewrites the
        // file and reopens the document, and a page handle held across that
        // would point into a document that no longer exists.
        drop(raw);
        if !form_plan.cuts.is_empty() {
            self.apply_form_cuts(request, &nested_covered, &form_paths)?;
        }

        self.touch();
        Ok(report)
    }
}

/// A run of words inside a form, as the survey found it: where it is drawn
/// from in the form's own space, and one page-space box per character.
struct NestedRun {
    /// Its position in the survey's object list.
    position: usize,
    /// The form it is drawn through — the page's own, or `None` for one the
    /// survey could not attribute.
    parent: Option<usize>,
    text: String,
    /// The text matrix's translation, in the form's coordinate space — what
    /// `content::placed` reports for the operator that draws it.
    origin: (f32, f32),
    /// Page-space glyph boxes, one per character, in the order the text page
    /// gave them.
    boxes: Vec<Rect>,
    /// Each character's loose box — its advance box, page space — one per
    /// character, for measuring what a cut has to keep.
    loose: Vec<Rect>,
    /// A character whose box could not be read: the run goes whole.
    unreadable: bool,
}

/// One link in the chain of forms a cut goes through, from the page down.
#[derive(Clone)]
struct Link {
    /// The resource name the container draws it by.
    name: Vec<u8>,
    /// The XObject's object number.
    number: u32,
    dict: crate::pdf::Dict,
    /// The form's content, decoded.
    decoded: Vec<u8>,
    /// The `Do` in the container's stream that draws it.
    do_at: usize,
    /// How many `Do`s in the container's stream draw it under this name.
    drawings_here: usize,
    /// How many places outside the container draw the same XObject.
    uses_elsewhere: usize,
}

impl Link {
    /// Whether anything but this one drawing shows the form.
    fn shared(&self) -> bool {
        self.drawings_here > 1 || self.uses_elsewhere > 0
    }
}

/// One innermost form's stream with the covered words cut out of it, and
/// the chain of forms it is reached through, ready to write.
struct FormCut {
    /// The innermost form's position in the survey's object list.
    form: usize,
    /// From the page's own form down to the one holding the words.
    chain: Vec<Link>,
    /// The innermost form's content, decoded, with the cuts made.
    edited: Vec<u8>,
    characters: usize,
    spilled: Vec<String>,
    /// How many other drawings show the same words, through any link of the
    /// chain. Above zero, the cut goes into private copies.
    elsewhere: usize,
}

/// What can be cut out of the page's forms, and what cannot.
#[derive(Default)]
struct FormPlan {
    cuts: Vec<FormCut>,
    /// Nested runs this pass could not place: reported as uncleared.
    refused: Vec<usize>,
}

/// A form's XObjects, resolved: name to number and whether it is a form.
fn xobjects_of(
    file: &crate::pdf::File<'_>,
    resources: Option<&crate::pdf::Object>,
) -> Vec<(Vec<u8>, u32, bool)> {
    use crate::pdf::Object;
    resources
        .and_then(|r| file.resolve(r).ok())
        .and_then(|r| r.as_dict().and_then(|d| d.get(b"XObject")).cloned())
        .and_then(|x| file.resolve(&x).ok())
        .and_then(|x| x.as_dict().cloned())
        .map(|dict| {
            dict.0
                .iter()
                .filter_map(|(name, entry)| {
                    let (number, _) = entry.as_reference()?;
                    let is_form = matches!(
                        file.object(number),
                        Ok(Object::Stream(ref d, _))
                            if d.get(b"Subtype").and_then(Object::as_name) == Some(&b"Form"[..])
                    );
                    Some((name.clone(), number, is_form))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The form `Do`s in a stream, in order, with their names.
fn form_dos_in(
    operations: &[crate::pdf::content::Operation],
    xobjects: &[(Vec<u8>, u32, bool)],
) -> Vec<(usize, Vec<u8>)> {
    use crate::pdf::Object;
    operations
        .iter()
        .enumerate()
        .filter_map(|(at, op)| match (op.operator.as_slice(), op.operands.first()) {
            (b"Do", Some(Object::Name(name)))
                if xobjects.iter().any(|(n, _, is_form)| n == name && *is_form) =>
            {
                Some((at, name.clone()))
            }
            _ => None,
        })
        .collect()
}

impl PdfiumDocument {
    /// Work out how the covered words inside the page's forms come out.
    ///
    /// **Pure with respect to the document**: reads `bytes`, changes nothing,
    /// so the survey can report exactly what the apply will do — the two run
    /// the same code on the same bytes.
    ///
    /// A form is found in the file by its path of ordinals (see
    /// `form_paths`): the n-th form `Do` of the page's stream, then the m-th
    /// form `Do` of that form's stream, and so on down to the form holding
    /// the words. Each link on the way records whether anything else draws
    /// it, because a cut into a form the file draws elsewhere has to go into
    /// a copy — and once a link is copied, every link below it must be too,
    /// or the copy and the original would share what was cut.
    fn plan_form_cuts(
        &self,
        bytes: &[u8],
        request: &Redaction,
        nested: &[NestedRun],
        form_paths: &HashMap<usize, Vec<usize>>,
    ) -> Result<FormPlan> {
        use crate::pdf::{content, Object};

        let file = crate::pdf::File::parse(bytes)?;
        let page = self.page_object(&file, request.page_index)?;
        let (stream, _) = self.page_content(&file, &page)?;
        let page_ops = content::parse(&stream)?;
        let page_xobjects = xobjects_of(&file, self.inherited(&file, &page, b"Resources")?.as_ref());

        let mut plan = FormPlan::default();
        let mut by_form: std::collections::BTreeMap<usize, Vec<&NestedRun>> = Default::default();
        for run in nested {
            match run.parent {
                Some(parent) => by_form.entry(parent).or_default().push(run),
                None => plan.refused.push(run.position),
            }
        }

        for (form, runs) in by_form {
            let Some(path) = form_paths.get(&form) else {
                plan.refused.extend(runs.iter().map(|r| r.position));
                continue;
            };

            // Down the path, one container at a time.
            let mut chain: Vec<Link> = Vec::new();
            let mut ops = page_ops.clone();
            let mut xobjects = page_xobjects.clone();
            let mut container_number: Option<u32> = None;
            let mut resolved = true;
            for &ordinal in path {
                let dos = form_dos_in(&ops, &xobjects);
                let Some((do_at, name)) = dos.get(ordinal).cloned() else {
                    resolved = false;
                    break;
                };
                let drawings_here = dos.iter().filter(|(_, n)| *n == name).count();
                let Some(&(_, number, _)) = xobjects.iter().find(|(n, _, _)| *n == name) else {
                    resolved = false;
                    break;
                };
                let Ok(Object::Stream(dict, range)) = file.object(number) else {
                    resolved = false;
                    break;
                };
                let Some(decoded) =
                    file.bytes().get(range).and_then(|raw| content::decode(&dict, raw))
                else {
                    resolved = false;
                    break;
                };
                let Ok(inner_ops) = content::parse(&decoded) else {
                    resolved = false;
                    break;
                };
                let uses_elsewhere = self.xobject_uses(&file, number, container_number, request.page_index);
                chain.push(Link { name, number, dict: dict.clone(), decoded, do_at, drawings_here, uses_elsewhere });
                ops = inner_ops;
                xobjects = xobjects_of(&file, dict.get(b"Resources"));
                container_number = Some(number);
            }
            if !resolved || chain.is_empty() {
                plan.refused.extend(runs.iter().map(|r| r.position));
                continue;
            }
            let innermost = chain.last().expect("checked");
            let form_ops = content::parse(&innermost.decoded)?;
            let placed = content::placed(&form_ops);

            // The innermost form's own fonts, or the page's where it has none.
            let fonts = innermost
                .dict
                .get(b"Resources")
                .and_then(|r| file.resolve(r).ok())
                .and_then(|r| r.as_dict().and_then(|d| d.get(b"Font")).cloned())
                .and_then(|f| file.resolve(&f).ok())
                .and_then(|f| f.as_dict().cloned())
                .or_else(|| self.page_fonts(&file, &page));
            let ctx = CutContext { file: &file, bytes, fonts, operations: &form_ops };
            let codes_in = |p: &content::Placed| -> usize {
                let width = p
                    .font
                    .as_ref()
                    .zip(ctx.fonts.as_ref())
                    .and_then(|(name, dict)| code_width(&file, dict, name))
                    .unwrap_or(1)
                    .max(1);
                content::pieces(&form_ops[p.origin.operation])
                    .iter()
                    .map(|piece| match piece {
                        content::Piece::Codes(bytes) => bytes.len() / width,
                        content::Piece::Kern(_) => 0,
                    })
                    .sum()
            };

            // **PDFium folds the form's `/Matrix` into what it reports for the
            // objects inside**, while the stream's own operators are written
            // before it. Measured: a form with `/Matrix [0.5 0 0 0.5 20 10]`
            // reported its words at half size and shifted, and nothing
            // matched. The origin goes back through the inverse before it is
            // looked for.
            let matrix_of = |dict: &crate::pdf::Dict| -> [f32; 6] {
                dict.get(b"Matrix")
                    .and_then(|m| match m {
                        Object::Array(items) if items.len() == 6 => {
                            let n: Vec<f32> = items.iter().filter_map(|i| i.as_f64()).map(|v| v as f32).collect();
                            (n.len() == 6).then(|| [n[0], n[1], n[2], n[3], n[4], n[5]])
                        }
                        _ => None,
                    })
                    .unwrap_or(IDENTITY_MATRIX)
            };
            let Some(unmatrix) = inverse(matrix_of(&innermost.dict)) else {
                plan.refused.extend(runs.iter().map(|r| r.position));
                continue;
            };

            const NEAR: f32 = 4.0;
            let mut cuts = Cuts::default();
            let mut handled = 0usize;
            for run in &runs {
                // The operators drawing it, from where its text matrix says it
                // starts — in the stream's own space, which is what both
                // sides speak once the form's matrix is taken back off.
                let (x, y) = run.origin;
                let want_x = x * unmatrix[0] + y * unmatrix[2] + unmatrix[4];
                let want_y = x * unmatrix[1] + y * unmatrix[3] + unmatrix[5];
                let start = placed
                    .iter()
                    .map(|p| {
                        let d = ((p.origin.x - want_x).powi(2) + (p.origin.y - want_y).powi(2)).sqrt();
                        (p, d)
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|(_, d)| *d <= NEAR)
                    .map(|(p, _)| p);
                let Some(start) = start else {
                    plan.refused.push(run.position);
                    continue;
                };
                let wanted = run.text.chars().count();
                let mut following: Vec<&content::Placed> = placed
                    .iter()
                    .filter(|p| p.line == start.line && p.origin.x >= start.origin.x - 0.5)
                    .collect();
                following.sort_by_key(|p| p.origin.operation);
                let mut parts: Vec<&content::Placed> = Vec::new();
                let mut covered = 0usize;
                for p in following {
                    parts.push(p);
                    covered += codes_in(p);
                    if covered >= wanted {
                        break;
                    }
                }
                if parts.is_empty() {
                    plan.refused.push(run.position);
                    continue;
                }

                let mine: Vec<(usize, Rect)> = if !run.unreadable && wanted > 0 && run.boxes.len() == wanted {
                    run.boxes.iter().copied().enumerate().collect()
                } else {
                    Vec::new()
                };
                let run_rect = run.boxes.iter().fold(None::<Rect>, |acc, b| {
                    Some(match acc {
                        None => *b,
                        Some(r) => Rect {
                            left: r.left.min(b.left),
                            top: r.top.min(b.top),
                            right: r.right.max(b.right),
                            bottom: r.bottom.max(b.bottom),
                        },
                    })
                });
                let run_rect = run_rect.unwrap_or(Rect { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 });
                Self::cut_run(&ctx, request, &run.text, &run_rect, &parts, mine, &run.loose, &mut cuts);
                handled += 1;
            }
            if handled == 0 {
                continue;
            }

            // **No sweep by origin here**, unlike the page path. The text
            // page reported every real character in the form and which
            // object drew it, so anything the area touches is already in
            // `nested`. A sweep would only add what `content::placed` cannot
            // place: an operator continuing another reports the origin of
            // the one before it, and on a real catalogue's footer that took
            // ` Lighting` for starting where `HSI` did.
            let Cuts { mut edits, mut cut_whole, spilled, characters } = cuts;
            cut_whole.sort_unstable();
            cut_whole.dedup();
            for index in &cut_whole {
                edits.push((form_ops[*index].span.clone(), Vec::new()));
            }
            let edited = content::splice(&innermost.decoded, &edits);

            let elsewhere = chain
                .iter()
                .map(|link| (link.drawings_here - 1) + link.uses_elsewhere)
                .sum();
            plan.cuts.push(FormCut { form, chain, edited, characters, spilled, elsewhere });
        }
        Ok(plan)
    }

    /// How many places outside one container draw an XObject: every page's
    /// resources but `page`'s when the container is the page (inherited ones
    /// resolved per page, so two pages sharing one resources object count
    /// twice), and every form's own but the container's.
    fn xobject_uses(
        &self,
        file: &crate::pdf::File<'_>,
        number: u32,
        container: Option<u32>,
        page: usize,
    ) -> usize {
        use crate::pdf::Object;
        let counts = |resources: Option<Object>| -> usize {
            resources
                .and_then(|r| r.as_dict().and_then(|d| d.get(b"XObject")).cloned())
                .and_then(|x| file.resolve(&x).ok())
                .and_then(|x| x.as_dict().cloned())
                .map(|dict| {
                    dict.0
                        .iter()
                        .filter(|(_, entry)| matches!(entry, Object::Reference(n, _) if *n == number))
                        .count()
                })
                .unwrap_or(0)
        };
        let mut uses = 0usize;
        for index in 0..self.page_count {
            if container.is_none() && index == page {
                continue;
            }
            if let Ok(page) = self.page_object(file, index) {
                uses += counts(self.inherited(file, &page, b"Resources").ok().flatten());
            }
        }
        for object in file.numbers() {
            if Some(object) == container {
                continue;
            }
            if let Ok(Object::Stream(dict, _)) = file.object(object) {
                if dict.get(b"Subtype").and_then(Object::as_name) == Some(&b"Form"[..]) {
                    uses += counts(dict.get(b"Resources").and_then(|r| file.resolve(r).ok()));
                }
            }
        }
        uses
    }

    /// Write the planned cuts into the file and reopen from it.
    ///
    /// The innermost form is rewritten in place when every link of its chain
    /// is drawn by this page alone. Otherwise the chain is copied from the
    /// first shared link down: each copy is a new object with the cut, or
    /// with its resources pointing at the copy below it, and the container
    /// above the first copy — a form in place, or the page — is pointed at
    /// it under the drawing's name, made fresh where the container draws the
    /// same form more than once under one name.
    fn apply_form_cuts(
        &mut self,
        request: &Redaction,
        nested: &[NestedRun],
        form_paths: &HashMap<usize, Vec<usize>>,
    ) -> Result<()> {
        use crate::pdf::{content, write_object, write_stream, Object};

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let bytes = self.readable_bytes()?;
        let plan = self.plan_form_cuts(&bytes, request, nested, form_paths)?;
        if plan.cuts.is_empty() {
            return Ok(());
        }
        let file = crate::pdf::File::parse(&bytes)?;
        let page_number = self.page_object_number(&file, request.page_index)?;
        let mut page_dict = self
            .page_object(&file, request.page_index)?
            .as_dict()
            .cloned()
            .ok_or_else(|| PdfError::InvalidArgument("the page is not a dictionary".into()))?;
        let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut extras: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut next = file.next_object_number()?;
        let mut page_changed = false;
        // The page's stream, spliced where a drawing is given a name of its
        // own; written back only if something was.
        let (page_stream, page_streams) = self.page_content(&file, &Object::Dict(page_dict.clone()))?;
        let page_ops = content::parse(&page_stream)?;
        let mut page_edits: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
        // Forms edited in place above a copy: their dictionaries change (a
        // resource entry) and possibly their streams (a renamed `Do`).
        let mut form_dict_edits: HashMap<u32, (crate::pdf::Dict, Vec<(std::ops::Range<usize>, Vec<u8>)>)> =
            HashMap::new();

        for cut in &plan.cuts {
            let depth = cut.chain.len();
            // Where the copying starts: the first shared link, if any.
            let copy_from = cut.chain.iter().position(Link::shared);

            // The innermost form's new body.
            let innermost = &cut.chain[depth - 1];
            let packed = content::encode(&cut.edited)?;
            let mut dict = innermost.dict.clone();
            dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");

            let Some(copy_from) = copy_from else {
                replacements.push((innermost.number, write_stream(&dict, &packed)));
                continue;
            };

            // From the innermost up to the first copied link: each becomes a
            // new object, pointed at by the one above under the drawing's
            // name. The link above `copy_from` is edited in place instead —
            // a form, or the page.
            let mut below: Option<(u32, Vec<u8>)> = None; // (new number, name it goes under)
            for level in (copy_from..depth).rev() {
                let link = &cut.chain[level];
                let copy = next;
                next += 1;
                let mut link_dict = if level == depth - 1 { dict.clone() } else { link.dict.clone() };
                let mut link_stream_edits: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
                if let Some((child_number, child_name)) = below.take() {
                    // This copy's resources point at the copy below it.
                    let child = &cut.chain[level + 1];
                    let fresh_name = Self::point_xobject(&file, &mut link_dict, &child.name, &child_name, child_number);
                    if let Some(fresh) = fresh_name {
                        let ops = content::parse(&link.decoded)?;
                        let Some(op) = ops.get(child.do_at) else {
                            return Err(PdfError::Internal("a form's Do moved between survey and cut".into()));
                        };
                        let mut operand = b"/".to_vec();
                        operand.extend_from_slice(&fresh);
                        operand.extend_from_slice(b" Do");
                        link_stream_edits.push((op.span.clone(), operand));
                    }
                }
                let body = if level == depth - 1 {
                    write_stream(&link_dict, &packed)
                } else {
                    let data = content::splice(&link.decoded, &link_stream_edits);
                    let packed = content::encode(&data)?;
                    link_dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
                    link_dict.remove(b"DecodeParms");
                    write_stream(&link_dict, &packed)
                };
                extras.push((copy, body));
                // Under a fresh name when the container draws this form more
                // than once under the one it has.
                let under = if link.drawings_here > 1 {
                    let mut fresh = b"PgfCut".to_vec();
                    fresh.extend_from_slice(copy.to_string().as_bytes());
                    fresh
                } else {
                    link.name.clone()
                };
                below = Some((copy, under));
            }
            let (copy_number, copy_name) = below.expect("at least one link was copied");
            let top = &cut.chain[copy_from];

            if copy_from == 0 {
                // The page draws it: this page's own resources point at the copy.
                if copy_name != top.name {
                    let Some(op) = page_ops.get(top.do_at) else {
                        return Err(PdfError::Internal("a form's Do moved between survey and cut".into()));
                    };
                    let mut operand = b"/".to_vec();
                    operand.extend_from_slice(&copy_name);
                    operand.extend_from_slice(b" Do");
                    page_edits.push((op.span.clone(), operand));
                }
                let (mut resources, owned_object) = match page_dict.get(b"Resources").cloned() {
                    Some(Object::Dict(own)) => (own, None),
                    Some(Object::Reference(n, _)) => {
                        let dict = file.object(n)?.as_dict().cloned().unwrap_or(crate::pdf::Dict(Vec::new()));
                        let shared = self.resources_uses(&file, n) > 1;
                        (dict, if shared { None } else { Some(n) })
                    }
                    _ => (
                        self.inherited(&file, &Object::Dict(page_dict.clone()), b"Resources")?
                            .and_then(|r| r.as_dict().cloned())
                            .unwrap_or(crate::pdf::Dict(Vec::new())),
                        None,
                    ),
                };
                let mut xobjects = resources
                    .get(b"XObject")
                    .and_then(|x| file.resolve(x).ok())
                    .and_then(|x| x.as_dict().cloned())
                    .unwrap_or(crate::pdf::Dict(Vec::new()));
                xobjects.set(&copy_name, Object::Reference(copy_number, 0));
                resources.set(b"XObject", Object::Dict(xobjects));
                match (page_dict.get(b"Resources").cloned(), owned_object) {
                    (Some(Object::Dict(_)), _) => {
                        page_dict.set(b"Resources", Object::Dict(resources));
                        page_changed = true;
                    }
                    (Some(Object::Reference(..)), Some(n)) => {
                        let mut body = Vec::new();
                        write_object(&mut body, &Object::Dict(resources));
                        replacements.push((n, body));
                    }
                    _ => {
                        let own = next;
                        next += 1;
                        let mut body = Vec::new();
                        write_object(&mut body, &Object::Dict(resources));
                        extras.push((own, body));
                        page_dict.set(b"Resources", Object::Reference(own, 0));
                        page_changed = true;
                    }
                }
            } else {
                // A form in place draws it: its resources point at the copy,
                // and its stream is spliced if the drawing needed a name.
                let container = &cut.chain[copy_from - 1];
                let entry = form_dict_edits
                    .entry(container.number)
                    .or_insert_with(|| (container.dict.clone(), Vec::new()));
                let fresh = Self::point_xobject(&file, &mut entry.0, &top.name, &copy_name, copy_number);
                if let Some(fresh) = fresh {
                    let ops = content::parse(&container.decoded)?;
                    let Some(op) = ops.get(top.do_at) else {
                        return Err(PdfError::Internal("a form's Do moved between survey and cut".into()));
                    };
                    let mut operand = b"/".to_vec();
                    operand.extend_from_slice(&fresh);
                    operand.extend_from_slice(b" Do");
                    entry.1.push((op.span.clone(), operand));
                }
            }
        }

        for (number, (dict, stream_edits)) in form_dict_edits {
            let link = plan
                .cuts
                .iter()
                .flat_map(|c| c.chain.iter())
                .find(|l| l.number == number)
                .expect("an edited container is on some chain");
            let data = content::splice(&link.decoded, &stream_edits);
            let packed = content::encode(&data)?;
            let mut dict = dict;
            dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((number, write_stream(&dict, &packed)));
        }
        if page_changed {
            let mut body = Vec::new();
            write_object(&mut body, &Object::Dict(page_dict));
            replacements.push((page_number, body));
        }
        if !page_edits.is_empty() {
            // As the page path writes it: everything into the first stream,
            // the others emptied, the dictionary untouched.
            let edited = content::splice(&page_stream, &page_edits);
            for (index, (number, dict)) in page_streams.iter().enumerate() {
                let data = if index == 0 { edited.clone() } else { Vec::new() };
                let packed = content::encode(&data)?;
                let mut dict = dict.clone();
                dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
                dict.remove(b"DecodeParms");
                replacements.push((*number, write_stream(&dict, &packed)));
            }
        }

        let rewritten = file.rewrite_adding(&replacements, &extras, &crate::pdf::Dict(Vec::new()))?;
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        self.rearm_security(was_secured, plus, permissions);
        self.touch();
        Ok(())
    }

    /// Point a form's `/Resources /XObject` entry for a child at `target`,
    /// under `under` — returning the name when it differs from `was`, which
    /// is when the child's `Do` has to be renamed to match.
    ///
    /// The XObject dictionary is made direct, with the one entry changed:
    /// a shared dictionary object must not be edited for one form's sake.
    fn point_xobject(
        file: &crate::pdf::File<'_>,
        dict: &mut crate::pdf::Dict,
        was: &[u8],
        under: &[u8],
        target: u32,
    ) -> Option<Vec<u8>> {
        use crate::pdf::Object;
        let mut resources = dict
            .get(b"Resources")
            .and_then(|r| file.resolve(r).ok())
            .and_then(|r| r.as_dict().cloned())
            .unwrap_or(crate::pdf::Dict(Vec::new()));
        let mut xobjects = resources
            .get(b"XObject")
            .and_then(|x| file.resolve(x).ok())
            .and_then(|x| x.as_dict().cloned())
            .unwrap_or(crate::pdf::Dict(Vec::new()));
        xobjects.set(under, Object::Reference(target, 0));
        resources.set(b"XObject", Object::Dict(xobjects));
        dict.set(b"Resources", Object::Dict(resources));
        (under != was).then(|| under.to_vec())
    }

    /// How many pages and forms name one resources object.
    fn resources_uses(&self, file: &crate::pdf::File<'_>, number: u32) -> usize {
        use crate::pdf::Object;
        let names_it = |dict: &crate::pdf::Dict| {
            matches!(dict.get(b"Resources"), Some(Object::Reference(n, _)) if *n == number)
        };
        file.numbers()
            .filter(|n| match file.object(*n) {
                Ok(Object::Dict(d)) => names_it(&d),
                Ok(Object::Stream(d, _)) => names_it(&d),
                _ => false,
            })
            .count()
    }
}


/// Everything a cut needs to know about the stream it is cutting from.
struct CutContext<'a> {
    file: &'a crate::pdf::File<'a>,
    bytes: &'a [u8],
    /// The `/Font` resources the stream's operators name.
    fonts: Option<crate::pdf::Dict>,
    operations: &'a [crate::pdf::content::Operation],
}

/// What cutting runs out of one content stream produced, gathered as it goes.
#[derive(Default)]
struct Cuts {
    /// Byte ranges of the stream, each with what replaces it.
    edits: Vec<(std::ops::Range<usize>, Vec<u8>)>,
    /// Operations to take out whole.
    cut_whole: Vec<usize>,
    /// Runs cut whole that reached beyond the area.
    spilled: Vec<String>,
    characters: usize,
}

impl PdfiumDocument {
    /// Cut one run's covered characters out of its operators — sliced where
    /// the codes can be told apart, whole where they cannot.
    ///
    /// **The one decision the page path and the form path share**, so that
    /// words drawn through a form XObject are cut by exactly the rule words
    /// on the page are. `parts` are the operators drawing the run, in order;
    /// `mine` is one page-space box per character the run reported, or empty
    /// where the boxes could not be matched to the characters.
    fn cut_run(
        ctx: &CutContext<'_>,
        request: &Redaction,
        run_text: &str,
        run_rect: &Rect,
        parts: &[&crate::pdf::content::Placed],
        mine: Vec<(usize, Rect)>,
        loose: &[Rect],
        cuts: &mut Cuts,
    ) {
        use crate::pdf::content;
        let widths: Vec<Option<usize>> = parts
            .iter()
            .map(|p| {
                p.font
                    .as_ref()
                    .zip(ctx.fonts.as_ref())
                    .and_then(|(name, dict)| code_width(ctx.file, dict, name))
            })
            .collect();
        let counts: Vec<usize> = parts
            .iter()
            .zip(&widths)
            .map(|(p, width)| {
                let w = width.unwrap_or(1).max(1);
                content::pieces(&ctx.operations[p.origin.operation])
                    .iter()
                    .map(|piece| match piece {
                        content::Piece::Codes(b) => b.len() / w,
                        content::Piece::Kern(_) => 0,
                    })
                    .sum::<usize>()
            })
            .collect();
        // **Which code produced which character**, from the font's own
        // `/ToUnicode` table rather than by counting.
        //
        // Counting only works while a code spells exactly one character.
        // Real files break that both ways — `02DB` spells `"fl"` on a
        // catalogue page, and a code can spell something PDFium leaves out
        // of its text entirely — after which a character index and a code
        // index are different numbers. Two earlier attempts guessed at the
        // difference as an offset and moved a line by three points.
        let wanted = run_text.chars().count();
        let spellings: Vec<Option<String>> = parts
            .iter()
            .zip(&widths)
            .flat_map(|(part, width)| {
                let map = part
                    .font
                    .as_ref()
                    .zip(ctx.fonts.as_ref())
                    .and_then(|(name, dict)| Self::font_to_unicode(ctx.file, ctx.bytes, dict, name));
                codes_of(&ctx.operations[part.origin.operation], width.unwrap_or(1))
                    .into_iter()
                    .map(move |code| {
                        map.as_ref().and_then(|m| m.get(&code)).map(String::clone)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        // Where a font carries no `/ToUnicode` — plenty do not — there is
        // nothing to align against, and the old assumption is the best
        // available: one code, one character, but **only** when the two
        // counts agree exactly. That is what the simple fixtures rely on,
        // and dropping it made a phrase there take its whole line again.
        let owner = align_codes(&spellings, &run_text).or_else(|| {
            (spellings.len() == wanted).then(|| (0..wanted).collect())
        });

        // Which characters the selection covers.
        let covered_chars: Vec<usize> = mine
            .iter()
            .enumerate()
            .filter(|(_, (_, r))| request.touches(r))
            .map(|(n, _)| n)
            .collect();

        let sliceable = widths.iter().all(Option::is_some)
            && owner.is_some()
            && !mine.is_empty()
            && !covered_chars.is_empty()
            && covered_chars.len() < mine.len();

        // Where each code's characters begin — and from that, how far each
        // code moved the pen: from where its characters start to where the
        // next code's do, measured across the glyphs rather than from font
        // metrics, so side bearings and character spacing are already in it.
        // Known whenever the codes align with the characters and the glyphs
        // were measured, whether or not the run is being sliced.
        let measured = widths.iter().all(Option::is_some) && owner.is_some() && !mine.is_empty();
        let first_char: std::collections::BTreeMap<usize, usize> = owner
            .as_ref()
            .map(|owner| {
                let mut first_char = std::collections::BTreeMap::new();
                for (character, code) in owner.iter().enumerate() {
                    if *code != usize::MAX {
                        first_char.entry(*code).or_insert(character);
                    }
                }
                first_char
            })
            .unwrap_or_default();
        let starts: Vec<(usize, usize)> = first_char.iter().map(|(c, ch)| (*c, *ch)).collect();
        let advance_of = |code: usize| -> f32 {
            let Some(position) = starts.iter().position(|(c, _)| *c == code) else {
                // Owns no character, so it moved the pen by nothing that
                // can be seen.
                return 0.0;
            };
            let from = starts[position].1;
            // **Loose boxes where they were read.** A glyph's own box stops
            // at its ink — short of the pen on both sides by a side bearing —
            // and an advance measured between ink edges is short by the
            // difference, so a fragment continuing the line landed to the
            // left of where it was. The loose box is the advance box: they
            // tile the line exactly. Measured: 2.4 points on `HSI`.
            let left_of = |i: usize| loose.get(mine[i].0).map(|r| r.left).unwrap_or(mine[i].1.left);
            let right_of = |i: usize| loose.get(mine[i].0).map(|r| r.right).unwrap_or(mine[i].1.right);
            match starts.get(position + 1) {
                Some((_, next)) => left_of(*next) - left_of(from),
                None => right_of(mine.len() - 1) - left_of(from),
            }
        };

        if sliceable {
            let owner = owner.expect("checked");
            let mut drop_codes: Vec<usize> = covered_chars
                .iter()
                .filter_map(|c| owner.get(*c).copied())
                // A character no code produced — PDFium's own trailing
                // space — takes nothing with it.
                .filter(|code| *code != usize::MAX)
                .collect();
            drop_codes.sort_unstable();
            drop_codes.dedup();

            // Codes that left nothing in the extracted text — a soft
            // hyphen, say — own no character to be selected, but one lying
            // inside the removed stretch has to go with it.
            if let (Some(&first), Some(&last)) = (drop_codes.first(), drop_codes.last()) {
                let inside: Vec<usize> = (first..=last)
                    .filter(|c| !first_char.contains_key(c))
                    .collect();
                drop_codes.extend(inside);
                drop_codes.sort_unstable();
                drop_codes.dedup();
            }

            // Each part is rebuilt from the codes dropped inside it,
            // renumbered to its own.
            let mut at = 0usize;
            for ((part, size), width) in parts.iter().zip(&counts).zip(&widths) {
                let range = at..at + size;
                let local: Vec<(usize, f32)> = drop_codes
                    .iter()
                    .filter(|c| range.contains(c))
                    .map(|c| (c - at, advance_of(*c)))
                    .collect();
                at += size;
                if local.is_empty() {
                    continue;
                }
                let operation = &ctx.operations[part.origin.operation];
                let font = part.font.clone().unwrap_or_default();
                cuts.edits.push((
                    operation.span.clone(),
                    content::without_codes(
                        operation,
                        &local,
                        &font,
                        part.size,
                        part.scale,
                        width.unwrap_or(1),
                    ),
                ));
            }
            cuts.characters += covered_chars.len();
        } else {
            cuts.characters += run_text.chars().count();
            if measured {
                // **Taken whole, but the space it took is kept.** A fragment
                // that continues the line in the same text object — `HSI`
                // then ` Lighting`, one operator each, which is how a design
                // program kerns — would otherwise slide left into the gap.
                // Each operator becomes a pen movement of exactly its own
                // width, drawing nothing. Measured on a real catalogue's
                // footer, where a run cut outright took the word after it.
                let mut at = 0usize;
                for (part, size) in parts.iter().zip(&counts) {
                    let gap: f32 = (at..at + size).map(advance_of).sum();
                    at += size;
                    let operation = &ctx.operations[part.origin.operation];
                    let font = part.font.clone().unwrap_or_default();
                    cuts.edits.push((
                        operation.span.clone(),
                        content::advance_only(&font, part.size, part.scale, gap),
                    ));
                }
            } else {
                for part in parts {
                    cuts.cut_whole.push(part.origin.operation);
                }
            }
            // Only when something went that was *not* asked for. A run
            // wholly inside the selection loses nothing extra by being cut
            // whole, and reporting it would tell the caller a line vanished
            // when it did not.
            let beyond = request.spills(run_rect, 0.5);
            if beyond {
                cuts.spilled.push(run_text.to_string());
            }
        }
    }
}

/// A text object's own string, read through an already-open text page.
fn object_text(
    bindings: &dyn PdfiumLibraryBindings,
    text_page: pdfium_render::prelude::FPDF_TEXTPAGE,
    object: FPDF_PAGEOBJECT,
) -> String {
    let wanted =
        unsafe { bindings.FPDFTextObj_GetText(object, text_page, std::ptr::null_mut(), 0) };
    if wanted <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; (wanted as usize / 2).max(1)];
    let written = unsafe {
        bindings.FPDFTextObj_GetText(object, text_page, buffer.as_mut_ptr() as *mut _, wanted)
    };
    let len = (written as usize / 2).saturating_sub(1).min(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Rebuild a run from the pieces of it that survived a redaction.
///
/// The first surviving piece **reuses the original object**, which is what keeps
/// everything nobody thought to copy — clipping path, marked content, character
/// and word spacing, text rise. Later pieces are new objects carrying font,
/// size, colour, render mode and position, and nothing else; a run split into
/// two by a redaction through its middle is rare enough that the asymmetry is
/// worth stating rather than engineering away.
///
/// Every piece is placed at **its own** left edge rather than at the run's
/// origin. Rebuilt from the origin, the tail of a split line would slide left
/// into the gap the redaction made, and the page would read as a sentence with
/// nothing taken out of it.
fn rewrite_run(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    page: FPDF_PAGE,
    original: FPDF_PAGEOBJECT,
    text: &str,
    segments: &[crate::document::redact::Segment],
) -> Result<()> {
    use crate::document::redact::slice;

    let Some((first, rest)) = segments.split_first() else {
        // Nothing survived, so there is nothing to rebuild. The caller plans
        // this as a removal, not a rewrite; treated as a no-op rather than an
        // error so an empty split cannot leave a half-written page.
        return Ok(());
    };

    let mut matrix = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
    unsafe { bindings.FPDFPageObj_GetMatrix(original, &mut matrix) };
    let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 255u32);
    let had_colour =
        unsafe { bindings.FPDFPageObj_GetFillColor(original, &mut r, &mut g, &mut b, &mut a) } != 0;
    let mut size = 0.0f32;
    unsafe { bindings.FPDFTextObj_GetFontSize(original, &mut size) };
    let render_mode = unsafe { bindings.FPDFTextObj_GetTextRenderMode(original) };
    let font = unsafe { bindings.FPDFTextObj_GetFont(original) };

    // The new objects are built **before** the original is touched, so a
    // failure part-way leaves a page that still says what it said.
    let mut built: Vec<FPDF_PAGEOBJECT> = Vec::new();
    for segment in rest {
        let piece = slice(text, segment);
        if piece.is_empty() {
            continue;
        }
        let object = unsafe { bindings.FPDFPageObj_CreateTextObj(document, font, size.max(1.0)) };
        if object.is_null() {
            for orphan in built {
                unsafe { bindings.FPDFPageObj_Destroy(orphan) };
            }
            return Err(PdfError::Pdfium(
                "the surviving text could not be rebuilt".into(),
            ));
        }

        let mut utf16: Vec<u16> = piece.encode_utf16().collect();
        utf16.push(0);
        // Only ever a subset of what the run already drew, so the font is
        // guaranteed to have the glyphs — the usual failure of `SetText`, a
        // subsetted font asked for a character the document never contained,
        // cannot arise here.
        if unsafe { bindings.FPDFText_SetText(object, utf16.as_ptr()) } == 0 {
            unsafe { bindings.FPDFPageObj_Destroy(object) };
            for orphan in built {
                unsafe { bindings.FPDFPageObj_Destroy(orphan) };
            }
            return Err(PdfError::Pdfium(
                "the surviving text could not be rebuilt".into(),
            ));
        }

        if had_colour {
            unsafe { bindings.FPDFPageObj_SetFillColor(object, r, g, b, a) };
        }
        unsafe { bindings.FPDFTextObj_SetTextRenderMode(object, render_mode) };
        let placed = FS_MATRIX { e: segment.left, ..matrix };
        unsafe { bindings.FPDFPageObj_SetMatrix(object, &placed) };
        built.push(object);
    }

    // The original, rewritten to its own surviving piece.
    let head = slice(text, first);
    let mut utf16: Vec<u16> = head.encode_utf16().collect();
    utf16.push(0);
    if unsafe { bindings.FPDFText_SetText(original, utf16.as_ptr()) } == 0 {
        for orphan in built {
            unsafe { bindings.FPDFPageObj_Destroy(orphan) };
        }
        return Err(PdfError::Pdfium(
            "the surviving text could not be rebuilt".into(),
        ));
    }
    // `FPDFText_SetText` writes the object afresh and carries no fill colour
    // with it: white text came back black, which on a dark banner means the
    // words vanish.
    if had_colour {
        unsafe { bindings.FPDFPageObj_SetFillColor(original, r, g, b, a) };
    }
    let placed = FS_MATRIX { e: first.left, ..matrix };
    unsafe { bindings.FPDFPageObj_SetMatrix(original, &placed) };

    for object in built {
        unsafe { bindings.FPDFPage_InsertObject(page, object) };
    }
    Ok(())
}

fn to_pdf_rect(space: &PageSpace, rect: &Rect) -> FS_RECTF {
    let (left, top) = space.to_pdf(rect.left, rect.top);
    let (right, bottom) = space.to_pdf(rect.right, rect.bottom);
    FS_RECTF {
        left: left.min(right),
        right: left.max(right),
        top: top.max(bottom),
        bottom: top.min(bottom),
    }
}

/// The matrix that places an image object — which draws into the unit
/// square — onto `rect` (already in PDF's y-up space, via [`to_pdf_rect`]),
/// turned `rotation_degrees` clockwise about `rect`'s own centre: stretch
/// the unit square to `rect`'s width and height, rotate about the middle of
/// that (so turning a signature spins it in place rather than flinging it
/// away from where it was clicked), then move it to `rect`'s position. Used
/// whenever a picture's placement changes without changing its pixels —
/// first placed, later moved, resized, or rotated.
///
/// **Clockwise, to match app space, not PDF space.** Angles everywhere
/// above this engine are clockwise with y increasing downward (see
/// `page_space` module doc) — the same sense a person turning a picture on
/// screen expects. PDF space is y-up, where the ordinary rotation matrix
/// turns counterclockwise for a positive angle; negating the angle here is
/// what keeps "turn it clockwise" meaning the same thing in both spaces,
/// checked in `rotating_turns_the_picture_clockwise_on_screen` by rendering
/// an asymmetric picture and finding its dark corner where a clockwise turn
/// puts it, not by trusting the arithmetic.
fn image_placement_matrix(rect: &FS_RECTF, rotation_degrees: f32) -> FS_MATRIX {
    let (w, h) = (rect.right - rect.left, rect.top - rect.bottom);
    if rotation_degrees == 0.0 {
        return FS_MATRIX { a: w, b: 0.0, c: 0.0, d: h, e: rect.left, f: rect.bottom };
    }
    let (cx, cy) = ((rect.left + rect.right) / 2.0, (rect.bottom + rect.top) / 2.0);
    let theta = -rotation_degrees.to_radians();
    let (s, c) = theta.sin_cos();
    FS_MATRIX {
        a: w * c,
        b: w * s,
        c: -h * s,
        d: h * c,
        e: cx - (w / 2.0) * c + (h / 2.0) * s,
        f: cy - (w / 2.0) * s - (h / 2.0) * c,
    }
}

/// The smallest rect containing every part of a mark.
///
/// Every annotation needs a `/Rect`, and PDFium will not compute one: a highlight
/// whose rect does not enclose its quad points, or ink whose rect does not
/// enclose its strokes, is clipped to the rect and partly or wholly invisible.
fn bounding_box(annotation: &Annotation) -> Option<Rect> {
    let mut bounds: Option<Rect> = None;
    let mut grow = |left: f32, top: f32, right: f32, bottom: f32| {
        bounds = Some(match bounds {
            None => Rect {
                left,
                top,
                right,
                bottom,
            },
            Some(b) => Rect {
                left: b.left.min(left),
                top: b.top.min(top),
                right: b.right.max(right),
                bottom: b.bottom.max(bottom),
            },
        });
    };

    match annotation {
        Annotation::Highlight { rects, .. }
        | Annotation::Underline { rects, .. }
        | Annotation::StrikeOut { rects, .. }
        | Annotation::Squiggly { rects, .. } => {
            for r in rects {
                grow(
                    r.left.min(r.right),
                    r.top.min(r.bottom),
                    r.left.max(r.right),
                    r.top.max(r.bottom),
                );
            }
        }
        Annotation::Ink { strokes, width, .. } => {
            // Half the nib either side, or the rect clips the stroke it draws.
            let pad = (width / 2.0).max(0.0);
            for stroke in strokes {
                for p in stroke {
                    grow(p.x - pad, p.y - pad, p.x + pad, p.y + pad);
                }
            }
        }
        Annotation::Note { rect, .. } => grow(
            rect.left.min(rect.right),
            rect.top.min(rect.bottom),
            rect.left.max(rect.right),
            rect.top.max(rect.bottom),
        ),
        // A glyph hangs above its own origin and a little below it, so the box
        // has to allow the size either side: taking the origins alone would give
        // a rectangle the height of the baseline and clip every letter away.
        Annotation::Text { glyphs, size, .. } => {
            for g in glyphs {
                grow(g.x - size, g.y - size, g.x + size, g.y + size);
            }
        }
        Annotation::Image { rect, .. } => grow(
            rect.left.min(rect.right),
            rect.top.min(rect.bottom),
            rect.left.max(rect.right),
            rect.top.max(rect.bottom),
        ),
        Annotation::Fill { outline, width, .. } => {
            let pad = (width / 2.0).max(0.0);
            for p in outline {
                grow(p.x - pad, p.y - pad, p.x + pad, p.y + pad);
            }
        }
        Annotation::Link { rect, .. } => grow(
            rect.left.min(rect.right),
            rect.top.min(rect.bottom),
            rect.left.max(rect.right),
            rect.top.max(rect.bottom),
        ),
    }
    bounds
}

impl PdfiumDocument {
    /// Write words into the page's own content, one text object per glyph.
    ///
    /// Real text, not a drawing of it: the point of the whole feature is that a
    /// reader can select it, search it and copy it out. `text_object_probe`
    /// established that this survives a save and comes back out of PDFium's own
    /// text extraction, rotated glyphs included.
    ///
    /// One object per glyph rather than one per string, because that is what
    /// curved text needs — every letter carries its own rotation — and because a
    /// second path for the straight case would be a second thing to get wrong for
    /// no difference anyone could see. PDFium is given the position and the turn;
    /// it is never asked where a letter should go.
    ///
    /// `FPDFPage_GenerateContent` at the end is not optional. Without it the
    /// objects exist in the document and not in the page's content stream:
    /// present in memory, absent from the file.
    fn write_text(&self, page: &RawPage, annotation: &Annotation) -> Result<()> {
        let Annotation::Text {
            font,
            font_asset,
            size,
            color,
            glyphs,
            id,
            restore,
            frame,
            frame_width,
            ..
        } = annotation
        else {
            return Err(PdfError::InvalidArgument("not a text annotation".into()));
        };

        let bindings = pdfium()?.bindings();
        let space = page.space()?;
        let document = self.document.handle();

        // Safety: the document and page handles are live for the call, and every
        // object created is either inserted into the page or the call fails.
        unsafe {
            // Two ways to get a font, and which one decides how every glyph
            // below is written.
            //
            // A standard-14 font is named, not embedded, and addressed by
            // character — free, tiny, and Latin-only. An asset font is a real
            // file that goes into the document, addressed by glyph id, which is
            // the only way to write a form that has no character of its own: a
            // joined Arabic letter, a Devanagari conjunct, a ligature.
            let embedded = match font_asset {
                Some(name) => Some(load_embedded_font(bindings, document, name, glyphs)?),
                None => None,
            };
            // The ids as they are *in the subset*, which is what has to be
            // written: subsetting renumbers everything that survives.
            let written_ids = embedded.as_ref().map(|(_, ids)| ids.clone());
            let loaded = match &embedded {
                Some((handle, _)) => *handle,
                None => {
                    let handle = bindings.FPDFText_LoadStandardFont(document, font);
                    if handle.is_null() {
                        return Err(PdfError::Pdfium(format!("font {font} would not load")));
                    }
                    handle
                }
            };

            let mut first = true;
            for (index, glyph) in glyphs.iter().enumerate() {
                let object = bindings.FPDFPageObj_CreateTextObj(document, loaded, *size);
                if object.is_null() {
                    return Err(PdfError::Pdfium("could not create a text object".into()));
                }

                if let Some(ids) = &written_ids {
                    // Charcodes, not characters. With Identity-H the code *is*
                    // the glyph id, which is what the shaper handed us and the
                    // only way to ask for a joined form.
                    let code = [ids.get(index).copied().unwrap_or(0)];
                    if bindings.FPDFText_SetCharcodes(object, code.as_ptr(), 1) == 0 {
                        return Err(PdfError::Pdfium("a glyph id was refused".into()));
                    }
                } else {
                    let encoded: Vec<u16> = glyph
                        .ch
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    if bindings.FPDFText_SetText(object, encoded.as_ptr()) == 0 {
                        return Err(PdfError::Pdfium("could not set a glyph's text".into()));
                    }
                }

                bindings.FPDFPageObj_SetFillColor(
                    object,
                    color.r as c_uint,
                    color.g as c_uint,
                    color.b as c_uint,
                    color.a as c_uint,
                );

                // The app measures y downwards from the top of the crop and turns
                // clockwise; PDF measures up from the bottom and turns the other
                // way. Both flips happen here, once, as they do for every other
                // mark — doing it anywhere else puts text on the wrong half of the
                // page while still looking right in the app.
                let placed = space.to_pdf(glyph.x, glyph.y);
                let (sin, cos) = (-glyph.radians).sin_cos();
                bindings.FPDFPageObj_Transform(
                    object,
                    cos as f64,
                    sin as f64,
                    -sin as f64,
                    cos as f64,
                    placed.0 as f64,
                    placed.1 as f64,
                );

                // Tagged, so the words can be found again after any number of
                // saves. Without this text stopped being a mark the moment it was
                // saved: the eraser could take the ring off a clouded caption and
                // not the words inside it.
                let mark = bindings.FPDFPageObj_AddMark(object, TEXT_MARK_NAME);
                if mark.is_null() {
                    return Err(PdfError::Pdfium("could not tag a text object".into()));
                }
                bindings.FPDFPageObjMark_SetIntParam(document, object, mark, TEXT_MARK_ID, *id);
                // Only on the first: the blob describes the whole caption, and a
                // copy of it on every letter would bloat the file for nothing.
                if first {
                    bindings.FPDFPageObjMark_SetStringParam(
                        document,
                        object,
                        mark,
                        TEXT_MARK_RESTORE,
                        restore,
                    );
                    first = false;
                }

                bindings.FPDFPage_InsertObject(page.handle, object);
            }

            // The ring around the words, if there is one. Page content like the
            // letters and tagged with the same id, so erasing the caption takes
            // both — as a separate annotation it came apart the moment the file
            // was reopened.
            if frame.len() >= 2 {
                let path = bindings.FPDFPageObj_CreateNewPath(
                    space.to_pdf(frame[0].x, frame[0].y).0,
                    space.to_pdf(frame[0].x, frame[0].y).1,
                );
                if path.is_null() {
                    return Err(PdfError::Pdfium("could not create the frame path".into()));
                }
                for point in &frame[1..] {
                    let placed = space.to_pdf(point.x, point.y);
                    bindings.FPDFPath_LineTo(path, placed.0, placed.1);
                }
                bindings.FPDFPath_Close(path);
                bindings.FPDFPageObj_SetStrokeColor(
                    path,
                    color.r as c_uint,
                    color.g as c_uint,
                    color.b as c_uint,
                    color.a as c_uint,
                );
                bindings.FPDFPageObj_SetStrokeWidth(path, frame_width.max(0.1));
                bindings.FPDFPath_SetDrawMode(path, 0, 1);

                let mark = bindings.FPDFPageObj_AddMark(path, TEXT_MARK_NAME);
                if mark.is_null() {
                    return Err(PdfError::Pdfium("could not tag the frame".into()));
                }
                bindings.FPDFPageObjMark_SetIntParam(document, path, mark, TEXT_MARK_ID, *id);
                bindings.FPDFPage_InsertObject(page.handle, path);
            }

            if bindings.FPDFPage_GenerateContent(page.handle) == 0 {
                return Err(PdfError::Pdfium(
                    "text was written but the page content was not regenerated".into(),
                ));
            }
        }

        Ok(())
    }

    /// Write one mark onto an already-open page.
    ///
    /// Split out from `add_annotation` so the page stays open for exactly this
    /// call and the error paths all close it.
    fn write_annotation(&self, page: &RawPage, annotation: &Annotation) -> Result<()> {
        let bindings = pdfium()?.bindings();
        let space = page.space()?;

        let subtype = match annotation {
            Annotation::Highlight { .. } => ANNOT_HIGHLIGHT,
            Annotation::Underline { .. } => ANNOT_UNDERLINE,
            Annotation::StrikeOut { .. } => ANNOT_STRIKEOUT,
            Annotation::Squiggly { .. } => ANNOT_SQUIGGLY,
            Annotation::Ink { .. } => ANNOT_INK,
            Annotation::Note { .. } => ANNOT_TEXT,
            Annotation::Image { .. } => ANNOT_STAMP,
            // `FPDFAnnot_IsObjectSupportedSubtype` names only ink and stamp as
            // supported for object attachment — a `/Polygon` shell built the
            // same way as a fill path silently refuses to accept its own
            // object. Sharing Image's subtype is exactly why `read_annotation`
            // tries the picture reader before the fill reader below: the two
            // are told apart by what they carry, not by which subtype they are.
            Annotation::Fill { .. } => ANNOT_STAMP,
            Annotation::Link { .. } => ANNOT_LINK,
            // Routed away in `add_annotation`: text is page content, not an
            // annotation, and there is no subtype that would make it one.
            Annotation::Text { .. } => {
                return Err(PdfError::InvalidArgument(
                    "text is page content, not an annotation".into(),
                ))
            }
        };

        // Safety: the page handle is live for the duration, and the annotation is
        // closed before returning on every path.
        let annot = unsafe { bindings.FPDFPage_CreateAnnot(page.handle, subtype) };
        if annot.is_null() {
            return Err(PdfError::Pdfium("could not create annotation".into()));
        }

        let outcome = self.fill_annotation(annot, annotation, &space);

        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        outcome
    }

    fn fill_annotation(
        &self,
        annot: FPDF_ANNOTATION,
        annotation: &Annotation,
        space: &PageSpace,
    ) -> Result<()> {
        let bindings = pdfium()?.bindings();

        let bounds = bounding_box(annotation)
            .ok_or_else(|| PdfError::InvalidArgument("annotation has no geometry".into()))?;
        let rect = to_pdf_rect(space, &bounds);
        unsafe { bindings.FPDFAnnot_SetRect(annot, &rect) };

        // A picture carries no `/C` colour at all — it is pixels, not ink —
        // and is built entirely differently: an image page object, appended
        // to the (Stamp) annotation PDFium just created, rather than anything
        // `FPDFAnnot_SetColor` or the ink/text-markup calls below know about.
        if let Annotation::Image { rgba, width, height, .. } = annotation {
            return self.fill_image_annotation(annot, &rect, rgba, *width, *height);
        }

        // No colour, no border — a link is invisible ink, clickable without
        // drawing a box around the words it covers. Handled here, before the
        // colour match below, the same way `Image` is handled above it.
        if let Annotation::Link { uri, .. } = annotation {
            unsafe {
                bindings.FPDFAnnot_SetBorder(annot, 0.0, 0.0, 0.0);
                if bindings.FPDFAnnot_SetURI(annot, uri) == 0 {
                    return Err(PdfError::Pdfium("could not set the link's address".into()));
                }
            }
            return Ok(());
        }

        let colour = match annotation {
            Annotation::Highlight { color, .. }
            | Annotation::Underline { color, .. }
            | Annotation::StrikeOut { color, .. }
            | Annotation::Squiggly { color, .. }
            | Annotation::Ink { color, .. }
            | Annotation::Note { color, .. }
            | Annotation::Text { color, .. } => *color,
            Annotation::Fill { stroke_color, .. } => *stroke_color,
            Annotation::Image { .. } => unreachable!("returned above"),
            Annotation::Link { .. } => unreachable!("returned above"),
        };
        unsafe {
            bindings.FPDFAnnot_SetColor(
                annot,
                COLORTYPE_COLOR,
                colour.r as c_uint,
                colour.g as c_uint,
                colour.b as c_uint,
                colour.a as c_uint,
            )
        };

        // The colour again, as a string on a key of our own.
        //
        // PDFium's `FPDFAnnot_GetColor` refuses to report `/C` for any annotation
        // that carries an appearance stream — which every mark this engine writes
        // does, because that is what makes other viewers draw it. So a mark saved
        // and reopened came back in the fallback colour: measured on a phone as a
        // red arrow that turned yellow the moment the file was reopened.
        //
        // Writing it a second time under a key PDFium will hand back verbatim
        // costs nine bytes and makes the round trip faithful. Anyone else's
        // annotations are unaffected, and still read through `/C`.
        set_annotation_colour_key(bindings, annot, colour);

        match annotation {
            // Unreachable: routed away in `add_annotation`, which sends text to
            // `write_text` before this function is ever reached.
            Annotation::Text { .. } => {}
            // Unreachable: handled above and returned before this match.
            Annotation::Image { .. } => unreachable!("returned above"),
            Annotation::Highlight { rects, .. }
            | Annotation::Underline { rects, .. }
            | Annotation::StrikeOut { rects, .. }
            | Annotation::Squiggly { rects, .. } => {
                for r in rects {
                    let pdf = to_pdf_rect(space, r);
                    // Quad points are the four corners in the order PDF expects:
                    // top-left, top-right, bottom-left, bottom-right. Not the
                    // winding order a reader would guess — bottom-left comes
                    // third, and getting it wrong produces a bow-tie shape.
                    let quad = FS_QUADPOINTSF {
                        x1: pdf.left,
                        y1: pdf.top,
                        x2: pdf.right,
                        y2: pdf.top,
                        x3: pdf.left,
                        y3: pdf.bottom,
                        x4: pdf.right,
                        y4: pdf.bottom,
                    };
                    unsafe { bindings.FPDFAnnot_AppendAttachmentPoints(annot, &quad) };
                }
            }
            Annotation::Ink { strokes, .. } => {
                for stroke in strokes {
                    if stroke.len() < 2 {
                        // A single point is not a stroke PDFium will draw, and it
                        // would silently produce an empty ink list rather than a dot.
                        continue;
                    }
                    let points: Vec<FS_POINTF> = stroke
                        .iter()
                        .map(|p| {
                            let (x, y) = space.to_pdf(p.x, p.y);
                            FS_POINTF { x, y }
                        })
                        .collect();
                    let added = unsafe {
                        bindings.FPDFAnnot_AddInkStroke(annot, points.as_ptr(), points.len() as _)
                    };
                    if added < 0 {
                        return Err(PdfError::Pdfium("could not add ink stroke".into()));
                    }
                }
            }
            Annotation::Fill { outline, fill_color, width, .. } => {
                self.build_fill_path(annot, outline, *fill_color, colour, *width, space)?;
            }
            Annotation::Note { contents, .. } => {
                unsafe { bindings.FPDFAnnot_SetStringValue_str(annot, "Contents", contents) };
            }
            Annotation::Link { .. } => unreachable!("returned above"),
        }

        Ok(())
    }

    /// The interior a `Fill` annotation carries and an `Ink` one cannot: a
    /// real filled path, given to the annotation the same way
    /// [`PdfiumDocument::fill_image_annotation`] gives it a picture — as a
    /// page object of its own, built through PDFium's path API rather than
    /// anything `/Vertices`-shaped, because the embedder API can read a
    /// polygon's vertices but never write them.
    fn build_fill_path(
        &self,
        annot: FPDF_ANNOTATION,
        outline: &[Point],
        fill_color: Color,
        stroke_color: Color,
        width: f32,
        space: &PageSpace,
    ) -> Result<()> {
        if outline.len() < 3 {
            return Err(PdfError::InvalidArgument(
                "a filled shape needs at least three points".into(),
            ));
        }
        let bindings = pdfium()?.bindings();

        let (x0, y0) = space.to_pdf(outline[0].x, outline[0].y);
        let path = unsafe { bindings.FPDFPageObj_CreateNewPath(x0, y0) };
        if path.is_null() {
            return Err(PdfError::Pdfium("could not create a fill path".into()));
        }
        for p in &outline[1..] {
            let (x, y) = space.to_pdf(p.x, p.y);
            unsafe { bindings.FPDFPath_LineTo(path, x, y) };
        }
        unsafe { bindings.FPDFPath_Close(path) };
        unsafe {
            bindings.FPDFPageObj_SetFillColor(
                path,
                fill_color.r as c_uint,
                fill_color.g as c_uint,
                fill_color.b as c_uint,
                fill_color.a as c_uint,
            );
            bindings.FPDFPageObj_SetStrokeColor(
                path,
                stroke_color.r as c_uint,
                stroke_color.g as c_uint,
                stroke_color.b as c_uint,
                stroke_color.a as c_uint,
            );
            bindings.FPDFPageObj_SetStrokeWidth(path, width);
            bindings.FPDFPath_SetDrawMode(path, FILLMODE_WINDING, 1);
        }
        if unsafe { bindings.FPDFAnnot_AppendObject(annot, path) } == 0 {
            return Err(PdfError::Pdfium("the fill could not be attached".into()));
        }
        unsafe {
            bindings.FPDFAnnot_SetColor(
                annot,
                COLORTYPE_INTERIOR,
                fill_color.r as c_uint,
                fill_color.g as c_uint,
                fill_color.b as c_uint,
                fill_color.a as c_uint,
            )
        };
        Ok(())
    }

    /// Build a picture into a `/Stamp` annotation PDFium has just created:
    /// one image page object, its pixels, positioned to fill `rect`.
    ///
    /// **Built through PDFium's own object API, not the byte-safe writer.**
    /// Every other page-content edit in this engine goes through
    /// `crate::pdf` so a signed document's bytes move only by appending — see
    /// `EditBase`. A placed-but-not-applied signature is different: it lives
    /// in `/Annots`, which PDFium already owns and re-serialises freely, so
    /// there is nothing here for that discipline to protect yet. It starts
    /// to matter the moment a signature is *applied* — see
    /// [`PdfiumDocument::apply_signatures`], which does go through the
    /// byte-safe writer, for exactly that reason.
    fn fill_image_annotation(
        &self,
        annot: FPDF_ANNOTATION,
        rect: &FS_RECTF,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        let bindings = pdfium()?.bindings();
        if width == 0 || height == 0 {
            return Err(PdfError::InvalidArgument("that picture has no size".into()));
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(rgba.len()) {
            return Err(PdfError::InvalidArgument(
                "that picture's pixels do not match its width and height".into(),
            ));
        }

        // BGRA, blue first — see `BITMAP_BGRA`. The `image` crate, and every
        // caller above this one, works in RGBA; swapped once here rather than
        // asked of all of them.
        let mut bgra = rgba.to_vec();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        let bitmap = unsafe {
            bindings.FPDFBitmap_CreateEx(
                width as c_int,
                height as c_int,
                BITMAP_BGRA,
                bgra.as_mut_ptr() as *mut c_void,
                (width as c_int) * 4,
            )
        };
        if bitmap.is_null() {
            return Err(PdfError::Pdfium("the picture could not be prepared".into()));
        }

        let object = unsafe { bindings.FPDFPageObj_NewImageObj(self.document.handle()) };
        if object.is_null() {
            unsafe { bindings.FPDFBitmap_Destroy(bitmap) };
            return Err(PdfError::Pdfium("could not create a picture object".into()));
        }

        // No page yet — the object is not on one until `FPDFAnnot_AppendObject`
        // below gives it to the annotation, and the bitmap call's own doc says
        // the page list "may be NULL" / "may be 0" for exactly this case.
        let set =
            unsafe { bindings.FPDFImageObj_SetBitmap(std::ptr::null_mut(), 0, object, bitmap) };
        unsafe { bindings.FPDFBitmap_Destroy(bitmap) };
        if set == 0 {
            return Err(PdfError::Pdfium("the picture could not be placed".into()));
        }

        // A freshly placed picture is never rotated — turning it is
        // something a person does afterward, through `rotate_image_signature`.
        let matrix = image_placement_matrix(rect, 0.0);
        if unsafe { bindings.FPDFPageObj_SetMatrix(object, &matrix) } == 0 {
            return Err(PdfError::Pdfium("the picture could not be positioned".into()));
        }

        if unsafe { bindings.FPDFAnnot_AppendObject(annot, object) } == 0 {
            return Err(PdfError::Pdfium("the picture could not be attached".into()));
        }

        Ok(())
    }
}

/// The mapping between PDF page space and the space everything above the engine
/// uses.
///
/// PDFium reports a page's size, and renders it, from the **CropBox** — but
/// `FPDFText_GetRect`, `FPDFAnnot_SetRect` and every other geometry call speak
/// **MediaBox** coordinates. Where the two differ, subtracting from the page
/// height is not a coordinate conversion; it is a conversion plus a silent
/// translation.
///
/// A real price list made the size of that plain:
///
/// ```text
///   MediaBox [0 0 595.276 841.89]
///   CropBox  [36 90 541.276 751.89]
/// ```
///
/// PDFium reports the page as 505.276 x 661.89 and draws the CropBox. Text at
/// `y = 698` — comfortably inside the crop — came back as `661.89 - 698 =
/// -36.43`, a *negative* distance from the top of the page. Every run on every
/// page of that document was recorded 90 pt too high and 36 pt too far right, so
/// highlights landed on blank paper and the eraser could not find what it drew.
///
/// The negative tops are what gave it away, and they only appear because this
/// inset is large. A crop inset of a few points yields marks that are merely
/// slightly wrong — far harder to notice, and just as broken.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageSpace {
    /// x of the crop's left edge, in PDF space.
    left: f32,
    /// y of the crop's *top* edge, in PDF space. Not the height.
    top: f32,
}

impl PageSpace {
    fn new(left: f32, top: f32) -> Self {
        PageSpace { left, top }
    }

    /// The origin-anchored case: no crop, or none that could be read.
    fn at_origin(page_height: f32) -> Self {
        PageSpace {
            left: 0.0,
            top: page_height,
        }
    }

    /// Read the box PDFium actually renders.
    ///
    /// Crop first, media second, then the origin-anchored assumption the old code
    /// made — a page is entitled to declare neither, and one that declares no crop
    /// is cropped to its MediaBox. Reached through the public boundaries API
    /// rather than the raw page handle, which keeps the vendored crate at the
    /// single patched line it has.
    fn for_page(page: &PdfPage, page_height: f32) -> Self {
        let fallback = PageSpace {
            left: 0.0,
            top: page_height,
        };

        let bounds = page
            .boundaries()
            .crop()
            .or_else(|_| page.boundaries().media());

        match bounds {
            Ok(b) => {
                let left = b.bounds.left().value;
                let top = b.bounds.top().value;
                if left.is_finite() && top.is_finite() {
                    PageSpace { left, top }
                } else {
                    fallback
                }
            }
            Err(_) => fallback,
        }
    }

    /// PDF space to ours: origin at the crop's top-left, y increasing downwards.
    fn to_top_left(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.left, self.top - y)
    }

    /// Ours back to PDF space. The exact inverse of [`PageSpace::to_top_left`].
    fn to_pdf(&self, x: f32, y: f32) -> (f32, f32) {
        (x + self.left, self.top - y)
    }
}

#[cfg(test)]
mod page_space_tests {
    use super::PageSpace;

    /// The real numbers from the price list that exposed this.
    ///
    /// MediaBox `[0 0 595.276 841.89]`, CropBox `[36 90 541.276 751.89]`. PDFium
    /// reports the page as 505.276 x 661.89 and returns text geometry in MediaBox
    /// space, so the crop's top edge — 751.89 — is the reference, not the height.
    fn price_list() -> PageSpace {
        PageSpace::new(36.0, 751.89)
    }

    #[test]
    fn a_run_inside_the_crop_lands_inside_the_page() {
        // The run that came back at -36.43 before the fix.
        let (x, y) = price_list().to_top_left(65.78, 698.32);

        assert!((x - 29.78).abs() < 0.01, "x was {x}");
        assert!((y - 53.57).abs() < 0.01, "y was {y}");
        assert!(y > 0.0, "a run inside the crop must not be above the page");
    }

    #[test]
    fn subtracting_the_page_height_is_what_produced_a_negative_top() {
        // Kept as an explicit statement of the old behaviour, so the difference is
        // visible rather than something you have to reconstruct from git history.
        let page_height = 661.89;
        let old = page_height - 698.32;
        assert!(old < 0.0, "the old conversion put this run above the page");

        let (_, new) = price_list().to_top_left(65.78, 698.32);
        assert!(
            (new - old - 90.0).abs() < 0.01,
            "the inset is exactly 90 pt"
        );
    }

    #[test]
    fn the_two_directions_are_exact_inverses() {
        // A mark is written through `to_pdf` and read back through `to_top_left`,
        // so any disagreement between them moves every saved annotation.
        let space = price_list();
        for &(x, y) in &[(0.0, 0.0), (100.0, 250.5), (505.276, 661.89)] {
            let (px, py) = space.to_pdf(x, y);
            let (rx, ry) = space.to_top_left(px, py);
            assert!((rx - x).abs() < 0.001, "x {x} -> {px} -> {rx}");
            assert!((ry - y).abs() < 0.001, "y {y} -> {py} -> {ry}");
        }
    }

    #[test]
    fn a_page_with_no_inset_behaves_exactly_as_before() {
        // The common case must not move: an origin-anchored page is what the old
        // code assumed, and it was right about those.
        let space = PageSpace::at_origin(842.0);
        let (x, y) = space.to_top_left(100.0, 800.0);

        assert_eq!(100.0, x);
        assert_eq!(42.0, y);
    }
}

// ------------------------------------------------------------- reading marks --

impl PdfiumDocument {
    /// Reconstruct one annotation, or `None` for a type this engine does not model.
    ///
    /// Skipping is the important behaviour. A page can carry form widgets, links
    /// and stamps that have no representation here, and guessing at them would be
    /// worse than ignoring them: the caller addresses annotations by PDFium's own
    /// index, so an unmodelled one simply has no entry rather than shifting every
    /// index after it.
    fn read_annotation(
        &self,
        annot: FPDF_ANNOTATION,
        space: &PageSpace,
    ) -> Result<Option<Annotation>> {
        let bindings = pdfium()?.bindings();
        let subtype = unsafe { bindings.FPDFAnnot_GetSubtype(annot) };

        let colour = self.read_colour(annot);

        let annotation = match subtype {
            // All four are text markup: the same quadrilaterals over the same
            // words, read the same way, and only the subtype says which mark a
            // reader draws.
            ANNOT_HIGHLIGHT | ANNOT_UNDERLINE | ANNOT_STRIKEOUT | ANNOT_SQUIGGLY => {
                let count = unsafe { bindings.FPDFAnnot_CountAttachmentPoints(annot) };
                let mut rects = Vec::new();
                for i in 0..count {
                    let mut quad = FS_QUADPOINTSF {
                        x1: 0.0,
                        y1: 0.0,
                        x2: 0.0,
                        y2: 0.0,
                        x3: 0.0,
                        y3: 0.0,
                        x4: 0.0,
                        y4: 0.0,
                    };
                    if unsafe { bindings.FPDFAnnot_GetAttachmentPoints(annot, i, &mut quad) } == 0 {
                        continue;
                    }
                    // The quad's corners are not in a guaranteed order, so the rect
                    // is taken from the extremes rather than from x1/y1 and x4/y4.
                    let xs = [quad.x1, quad.x2, quad.x3, quad.x4];
                    let ys = [quad.y1, quad.y2, quad.y3, quad.y4];
                    let (left, top) = space.to_top_left(
                        xs.iter().cloned().fold(f32::INFINITY, f32::min),
                        ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
                    );
                    let (right, bottom) = space.to_top_left(
                        xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
                        ys.iter().cloned().fold(f32::INFINITY, f32::min),
                    );
                    rects.push(Rect {
                        left,
                        top,
                        right,
                        bottom,
                    });
                }
                if rects.is_empty() {
                    return Ok(None);
                }
                match subtype {
                    ANNOT_UNDERLINE => Annotation::Underline { rects, color: colour },
                    ANNOT_STRIKEOUT => Annotation::StrikeOut { rects, color: colour },
                    ANNOT_SQUIGGLY => Annotation::Squiggly { rects, color: colour },
                    _ => Annotation::Highlight { rects, color: colour },
                }
            }

            ANNOT_INK => {
                let paths = unsafe { bindings.FPDFAnnot_GetInkListCount(annot) };
                let mut strokes = Vec::new();
                for path in 0..paths {
                    // Sized first with a null buffer, as every counted PDFium
                    // getter wants: asking for the length and the data in one call
                    // is what silently truncates a long stroke.
                    let len = unsafe {
                        bindings.FPDFAnnot_GetInkListPath(annot, path, std::ptr::null_mut(), 0)
                    };
                    if len == 0 {
                        continue;
                    }
                    let mut raw = vec![FS_POINTF { x: 0.0, y: 0.0 }; len as usize];
                    let written = unsafe {
                        bindings.FPDFAnnot_GetInkListPath(annot, path, raw.as_mut_ptr(), len)
                    };
                    raw.truncate(written as usize);

                    let stroke: Vec<Point> = raw
                        .iter()
                        .map(|p| {
                            let (x, y) = space.to_top_left(p.x, p.y);
                            Point { x, y }
                        })
                        .collect();
                    if stroke.len() >= 2 {
                        strokes.push(stroke);
                    }
                }
                if strokes.is_empty() {
                    return Ok(None);
                }
                Annotation::Ink {
                    strokes,
                    color: colour,
                    // Not recorded on the annotation itself in a form this engine
                    // writes or reads; the border width lives in /BS, which PDFium
                    // does not expose. The nib is cosmetic on read-back — the
                    // strokes are what a hit test uses.
                    width: DEFAULT_INK_WIDTH_POINTS,
                }
            }

            ANNOT_TEXT => {
                let mut rect = FS_RECTF {
                    left: 0.0,
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                };
                if unsafe { bindings.FPDFAnnot_GetRect(annot, &mut rect) } == 0 {
                    return Ok(None);
                }
                let (left, top) = space.to_top_left(rect.left, rect.top);
                let (right, bottom) = space.to_top_left(rect.right, rect.bottom);
                Annotation::Note {
                    rect: Rect {
                        left: left.min(right),
                        top: top.min(bottom),
                        right: left.max(right),
                        bottom: top.max(bottom),
                    },
                    contents: read_annotation_string(annot, "Contents").unwrap_or_default(),
                    color: colour,
                }
            }

            // A picture and a fill are both a `/Stamp` carrying one page
            // object; which this is turns on whether that object is an image
            // or a path, not on the subtype.
            ANNOT_STAMP => match self
                .read_image_annotation(annot, space)
                .or_else(|| self.read_fill_annotation(annot, space))
            {
                Some(mark) => mark,
                // Not one this engine placed — some other stamp, or a shape
                // this cannot read back. Left exactly as it is, like every
                // other kind below.
                None => return Ok(None),
            },

            ANNOT_LINK => match self.read_link_annotation(annot, space) {
                Some(mark) => mark,
                // A `/GoTo` or anything else this engine did not write —
                // left exactly as it is, like every other kind above.
                None => return Ok(None),
            },

            // Widgets, everything else: left exactly as they are.
            _ => return Ok(None),
        };

        Ok(Some(annotation))
    }

    /// A `/Link` annotation's own address, read back — the inverse of
    /// `fill_annotation`'s `Annotation::Link` branch. `None` for anything
    /// but a plain `/URI` action, which is the only kind this engine ever
    /// writes.
    fn read_link_annotation(&self, annot: FPDF_ANNOTATION, space: &PageSpace) -> Option<Annotation> {
        let bindings = pdfium().ok()?.bindings();
        let link = unsafe { bindings.FPDFAnnot_GetLink(annot) };
        if link.is_null() {
            return None;
        }
        let action = unsafe { bindings.FPDFLink_GetAction(link) };
        if action.is_null() || unsafe { bindings.FPDFAction_GetType(action) } != PDFACTION_URI {
            return None;
        }

        let document = self.document.handle();
        let needed =
            unsafe { bindings.FPDFAction_GetURIPath(document, action, std::ptr::null_mut(), 0) };
        if needed == 0 {
            return None;
        }
        let mut buffer = vec![0u8; needed as usize];
        let written = unsafe {
            bindings.FPDFAction_GetURIPath(document, action, buffer.as_mut_ptr() as *mut c_void, needed)
        };
        if written == 0 {
            return None;
        }
        buffer.truncate(written as usize);
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        let uri = String::from_utf8(buffer).ok()?;

        let mut rect = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
        if unsafe { bindings.FPDFAnnot_GetRect(annot, &mut rect) } == 0 {
            return None;
        }
        let (left, top) = space.to_top_left(rect.left, rect.top);
        let (right, bottom) = space.to_top_left(rect.right, rect.bottom);
        Some(Annotation::Link {
            rect: Rect {
                left: left.min(right),
                top: top.min(bottom),
                right: left.max(right),
                bottom: top.max(bottom),
            },
            uri,
        })
    }

    /// A `/Stamp` annotation's picture, read back — the inverse of
    /// [`PdfiumDocument::fill_image_annotation`]. `None` for a stamp that is
    /// not exactly one image object, which is not one this engine placed.
    fn read_image_annotation(&self, annot: FPDF_ANNOTATION, space: &PageSpace) -> Option<Annotation> {
        let bindings = pdfium().ok()?.bindings();
        if unsafe { bindings.FPDFAnnot_GetObjectCount(annot) } != 1 {
            return None;
        }
        let object = unsafe { bindings.FPDFAnnot_GetObject(annot, 0) };
        if object.is_null()
            || unsafe { bindings.FPDFPageObj_GetType(object) }
                != pdfium_render::prelude::FPDF_PAGEOBJ_IMAGE as i32
        {
            return None;
        }

        let bitmap = unsafe { bindings.FPDFImageObj_GetBitmap(object) };
        if bitmap.is_null() {
            return None;
        }
        let width = unsafe { bindings.FPDFBitmap_GetWidth(bitmap) };
        let height = unsafe { bindings.FPDFBitmap_GetHeight(bitmap) };
        let stride = unsafe { bindings.FPDFBitmap_GetStride(bitmap) };
        let format = unsafe { bindings.FPDFBitmap_GetFormat(bitmap) };
        let rgba = if width > 0 && height > 0 && stride > 0 {
            let buffer = unsafe { bindings.FPDFBitmap_GetBuffer_as_vec(bitmap) };
            straight_rgba(&buffer, width as usize, height as usize, stride as usize, format)
        } else {
            None
        };
        unsafe { bindings.FPDFBitmap_Destroy(bitmap) };
        let rgba = rgba?;

        let mut rect = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
        if unsafe { bindings.FPDFAnnot_GetRect(annot, &mut rect) } == 0 {
            return None;
        }
        let (left, top) = space.to_top_left(rect.left, rect.top);
        let (right, bottom) = space.to_top_left(rect.right, rect.bottom);

        Some(Annotation::Image {
            rect: Rect {
                left: left.min(right),
                top: top.min(bottom),
                right: left.max(right),
                bottom: top.max(bottom),
            },
            rgba,
            width: width as u32,
            height: height as u32,
        })
    }

    /// A `Fill` annotation's boundary and colours, read back — the inverse of
    /// [`PdfiumDocument::build_fill_path`]. `None` for a polygon that is not
    /// exactly one path object, which is not one this engine placed.
    fn read_fill_annotation(&self, annot: FPDF_ANNOTATION, space: &PageSpace) -> Option<Annotation> {
        let bindings = pdfium().ok()?.bindings();
        if unsafe { bindings.FPDFAnnot_GetObjectCount(annot) } != 1 {
            return None;
        }
        let object = unsafe { bindings.FPDFAnnot_GetObject(annot, 0) };
        if object.is_null()
            || unsafe { bindings.FPDFPageObj_GetType(object) }
                != pdfium_render::prelude::FPDF_PAGEOBJ_PATH as i32
        {
            return None;
        }

        let count = unsafe { bindings.FPDFPath_CountSegments(object) };
        if count <= 0 {
            return None;
        }
        let mut outline = Vec::new();
        for i in 0..count {
            let segment = unsafe { bindings.FPDFPath_GetPathSegment(object, i) };
            if segment.is_null() {
                continue;
            }
            let (mut x, mut y) = (0.0f32, 0.0f32);
            if unsafe { bindings.FPDFPathSegment_GetPoint(segment, &mut x, &mut y) } == 0 {
                continue;
            }
            let (px, py) = space.to_top_left(x, y);
            outline.push(Point { x: px, y: py });
        }
        if outline.len() < 3 {
            return None;
        }

        let (mut fr, mut fg, mut fb, mut fa) = (0u32, 0u32, 0u32, 0u32);
        let fill_color = if unsafe {
            bindings.FPDFPageObj_GetFillColor(object, &mut fr, &mut fg, &mut fb, &mut fa)
        } != 0
        {
            Color { r: fr as u8, g: fg as u8, b: fb as u8, a: fa as u8 }
        } else {
            DEFAULT_MARK_COLOUR
        };

        let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
        let stroke_color = if unsafe {
            bindings.FPDFPageObj_GetStrokeColor(object, &mut sr, &mut sg, &mut sb, &mut sa)
        } != 0
        {
            Color { r: sr as u8, g: sg as u8, b: sb as u8, a: sa as u8 }
        } else {
            fill_color
        };

        let mut width = DEFAULT_INK_WIDTH_POINTS;
        let mut w = 0.0f32;
        if unsafe { bindings.FPDFPageObj_GetStrokeWidth(object, &mut w) } != 0 && w > 0.0 {
            width = w;
        }

        Some(Annotation::Fill { outline, fill_color, stroke_color, width })
    }

    fn read_colour(&self, annot: FPDF_ANNOTATION) -> Color {
        let Ok(pdfium) = pdfium() else {
            return DEFAULT_MARK_COLOUR;
        };
        // Our own key first: PDFium will not report `/C` once an appearance
        // stream exists, and every mark this engine writes has one.
        if let Some(colour) = annotation_colour_key(annot) {
            return colour;
        }

        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        let ok = unsafe {
            pdfium.bindings().FPDFAnnot_GetColor(
                annot,
                COLORTYPE_COLOR,
                &mut r,
                &mut g,
                &mut b,
                &mut a,
            )
        } != 0;

        if ok {
            Color {
                r: r as u8,
                g: g as u8,
                b: b as u8,
                a: a as u8,
            }
        } else {
            // An annotation is allowed to carry no colour at all, in which case a
            // viewer picks one. Returning a visible default beats returning
            // transparent black, which would read back as an invisible mark.
            DEFAULT_MARK_COLOUR
        }
    }
}

/// Write a mark's colour to a key of this engine's own.
///
/// See the call site: PDFium will not report `/C` for an annotation that has an
/// appearance stream, so the colour has to be recorded somewhere it *will* hand
/// back. `AARRGGBB` in hex, which is how the app holds a colour anyway.
fn set_annotation_colour_key(
    bindings: &dyn pdfium_render::prelude::PdfiumLibraryBindings,
    annot: FPDF_ANNOTATION,
    colour: Color,
) {
    let packed = format!(
        "{:02X}{:02X}{:02X}{:02X}",
        colour.a, colour.r, colour.g, colour.b
    );
    let mut value: Vec<u16> = packed.encode_utf16().collect();
    value.push(0);
    unsafe {
        bindings.FPDFAnnot_SetStringValue(annot, COLOUR_KEY, value.as_ptr());
    }
}

/// Read back what [`set_annotation_colour_key`] wrote, if it is there.
///
/// Only this engine's own marks carry it. Anything else falls through to `/C`,
/// which is the right answer for an annotation somebody else wrote.
fn annotation_colour_key(annot: FPDF_ANNOTATION) -> Option<Color> {
    packed_colour(&read_annotation_string(annot, COLOUR_KEY)?)
}

/// `AARRGGBB` as this engine writes it, or nothing.
///
/// Eight *hex digits*, not eight bytes: a value somebody else wrote under the
/// key with a multi-byte character in it is the same length in bytes and
/// panicked on the slice — on `marks`, `status`, or any click on the page.
/// Found by audit.
fn packed_colour(packed: &str) -> Option<Color> {
    if packed.len() != 8 || !packed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&packed[at..at + 2], 16).ok();
    Some(Color {
        a: byte(0)?,
        r: byte(2)?,
        g: byte(4)?,
        b: byte(6)?,
    })
}

/// The key this engine records a mark's colour under.
const COLOUR_KEY: &str = "PagifyColor";

/// The key that says an ink annotation is a signature, not a drawing.
///
/// Its value is what the signature was called when it was placed. Written into
/// the file rather than remembered in the program, so a document closed and
/// reopened still knows which of its ink is somebody's name — see
/// [`crate::document::SignatureMark`].
const SIGNATURE_KEY: &str = "PagifySignature";

/// The key a picture signature's original, alpha-bearing pixels are found
/// under in [`PdfiumDocument::image_alpha`] — see that field's doc for why
/// they are not simply the annotation's own pixels. Written the same way
/// [`SIGNATURE_KEY`] is, but the value only ever means anything within the
/// session that wrote it; nothing reads this key back across a save.
const ALPHA_ID_KEY: &str = "PagifyAlphaId";

/// The angle a placed picture signature is rotated by, clockwise in
/// degrees, written the same way [`SIGNATURE_KEY`] is — but unlike that
/// key, this one is entirely this engine's own: no PDF viewer reads it, and
/// nothing about the annotation's own `/Rect` or the image object's own
/// bounds says it is there. It exists because `Annotation::Image` (the
/// wire shape shared with Kotlin and Swift) has no rotation field, and does
/// not need one just for this engine to remember an angle between one call
/// and the next — see [`image_placement_matrix`] for where the angle
/// actually takes effect. Absent (read back as 0.0) for every picture
/// placed before rotation existed, and for every one never rotated since.
const ROTATION_KEY: &str = "PagifyRotation";

/// Read [`ROTATION_KEY`] off an annotation, or 0.0 if it was never set.
fn read_rotation(annot: FPDF_ANNOTATION) -> f32 {
    read_annotation_string(annot, ROTATION_KEY).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0)
}

/// Read a UTF-16LE string value off an annotation.
fn read_annotation_string(annot: FPDF_ANNOTATION, key: &str) -> Option<String> {
    let bindings = pdfium().ok()?.bindings();

    // Length first, in bytes, including the terminator.
    let bytes = unsafe { bindings.FPDFAnnot_GetStringValue(annot, key, std::ptr::null_mut(), 0) };
    if bytes <= 2 {
        return None;
    }

    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    unsafe { bindings.FPDFAnnot_GetStringValue(annot, key, buffer.as_mut_ptr(), bytes) };

    // Drop the trailing NUL before decoding, or every value gains a stray char.
    if let Some(end) = buffer.iter().position(|&c| c == 0) {
        buffer.truncate(end);
    }
    Some(String::from_utf16_lossy(&buffer))
}

/// Nib width used for ink read back out of a document.
const DEFAULT_INK_WIDTH_POINTS: f32 = 2.0;

/// Shown for a mark whose own colour cannot be read.
const DEFAULT_MARK_COLOUR: Color = Color {
    r: 255,
    g: 214,
    b: 0,
    a: 255,
};

/// Close the cross-reference stream that PDFium leaves open on an incremental save.
///
/// PDFium appends its trailing xref *stream* without an `endobj`:
///
/// ```text
///   169 0 obj <</Type/XRef ... /Length 25>>stream
///   <binary>
///   endstream
///   startxref
///   141073
///   %%EOF
/// ```
///
/// An indirect object has to be closed, so the file is malformed. It only happens
/// on documents that use cross-reference *streams* — PDF 1.5 and later, which is
/// most things — and never on the classic `xref` tables the small fixtures use.
///
/// PDFium reads its own output regardless, silently reconstructing the table, and
/// that is why every round-trip test in this crate passed while the app was
/// writing damaged files. `qpdf --check` on a real document saved by the app:
///
/// ```text
///   WARNING: expected endobj (xref stream: object 169 0)
///   WARNING: file is damaged
///   WARNING: Attempting to reconstruct cross-reference table
/// ```
///
/// against a clean report on the same document before the edit, and clean reports
/// on every full-copy save. Incremental was the only path that did it, and it does
/// it even with no edit at all.
///
/// Inserting the keyword is safe with respect to offsets, which is the only thing
/// that could make this worse than the bug. Everything the xref stream points at
/// lies *before* it, and `startxref` names the stream's own offset — also before
/// the insertion. Nothing that is pointed at moves.
fn close_trailing_xref_object(bytes: Vec<u8>) -> Vec<u8> {
    const ENDSTREAM: &[u8] = b"endstream";
    const STARTXREF: &[u8] = b"startxref";
    const ENDOBJ: &[u8] = b"endobj";

    let Some(startxref) = find_last(&bytes, STARTXREF) else {
        // No trailer to speak of. Not this function's business to invent one.
        return bytes;
    };
    // Only when the trailing cross-reference really is a *stream*. Everything
    // below inserts bytes, and inserting is safe only because nothing the
    // trailing stream's own table points at lies after it.
    //
    // The first version asked "is there an `endstream` before `startxref`?" and
    // read a no as "classic table". That is a different question: a document with
    // a classic table still has content streams, so the answer was yes, and the
    // repair went to work on an ordinary object in the middle of the file —
    // pushing the table ten bytes past where `startxref` said it was. The
    // fixtures could not catch it, because every one of them was a blank page
    // with no stream in it at all.
    //
    // So ask the file rather than infer: follow `startxref` and look at what is
    // there. A classic table announces itself with the `xref` keyword.
    if !trailing_xref_is_a_stream(&bytes, startxref) {
        return bytes;
    }

    let Some(endstream) = find_last(&bytes[..startxref], ENDSTREAM) else {
        return bytes;
    };

    let gap = &bytes[endstream + ENDSTREAM.len()..startxref];
    let mut bytes = if contains(gap, ENDOBJ) {
        bytes
    } else {
        let mut fixed = Vec::with_capacity(bytes.len() + ENDOBJ.len() + 2);
        fixed.extend_from_slice(&bytes[..endstream + ENDSTREAM.len()]);
        fixed.extend_from_slice(b"\nendobj\n");
        fixed.extend_from_slice(&bytes[startxref..]);
        fixed
    };

    // Second defect, same save, and the one that actually stops a reader finding
    // the table. PDFium writes the dictionary without `/Type /XRef`:
    //
    //     169 0 obj <</Info 16 0 R /Root 19 0 R /Size 170/Prev 136467/...>>stream
    //
    // where the same document's own xref stream, written by whatever produced it,
    // reads `<</Type /XRef/W[1 4 2]/Index[0 169]/...`. The key is required — a
    // cross-reference stream is identified by it — so without it `startxref` names
    // an offset that holds an object no reader will accept as a table, and qpdf
    // reports `xref not found` at exactly the right offset.
    if let Some(dict) = trailing_dictionary_start(&bytes) {
        // Asked of the dictionary alone: from its `<<` to the `stream` keyword
        // that follows it, never the binary table after that, where the bytes
        // `/Type` could turn up by chance.
        let dictionary_end = bytes[dict..].windows(6).position(|w| w == b"stream").map_or(dict, |n| dict + n);
        if !contains(&bytes[dict..dictionary_end], b"/Type") {
            let mut typed = Vec::with_capacity(bytes.len() + 12);
            typed.extend_from_slice(&bytes[..dict]);
            typed.extend_from_slice(b"/Type/XRef");
            typed.extend_from_slice(&bytes[dict..]);
            bytes = typed;
        }
    }

    bytes
}

impl PdfiumDocument {
    /// Turn one run of words that shares its text object with others — which a
    /// matrix written round it cannot do, because a transform is not allowed
    /// inside a text object — through PDFium's own object, which treats every
    /// show-text as a thing of its own.
    ///
    /// **Guarded the way every other change through PDFium here is.** Writing the
    /// page back (`FPDFPage_GenerateContent`) re-emits all of it, and on some
    /// pages that has scrambled what nobody touched. So what the rest of the page
    /// says is read before and after, and if it differs the document is put back
    /// exactly as it was and the turn is refused.
    fn rotate_text_object_via_pdfium(&mut self, page_index: usize, object: usize, page_matrix: [f32; 6]) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index)
            .map_err(|_| PdfError::InvalidArgument(format!("page index {page_index} is out of range")))?;
        let rest = |runs: &[crate::document::TextRun]| -> Vec<String> {
            let mut out: Vec<String> =
                runs.iter().filter(|r| r.object != object).map(|r| r.text.trim().to_string()).collect();
            out.sort();
            out
        };
        let untouched = rest(&self.text_runs(page_index)?);
        let snapshot = self.document.save_to_bytes().map_err(|e| PdfError::Pdfium(e.to_string()))?;

        {
            let raw = RawPage::open(self.document.handle(), page_number)?;
            let bindings = pdfium()?.bindings();
            let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
            let index = i32::try_from(object)
                .map_err(|_| PdfError::InvalidArgument(format!("object {object} is out of range")))?;
            if index < 0 || index >= count {
                return Err(PdfError::InvalidArgument(format!("page {} has no object {object}", page_index + 1)));
            }
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, index) };
            if handle.is_null()
                || unsafe { bindings.FPDFPageObj_GetType(handle) } != pdfium_render::prelude::FPDF_PAGEOBJ_TEXT as i32
            {
                return Err(PdfError::InvalidArgument("that is not text".into()));
            }
            let [a, b, c, d, e, f] = page_matrix;
            unsafe { bindings.FPDFPageObj_Transform(handle, a as f64, b as f64, c as f64, d as f64, e as f64, f as f64) };
            if unsafe { bindings.FPDFPage_GenerateContent(raw.handle) } == 0 {
                return Err(PdfError::Pdfium("the page could not be rewritten".into()));
            }
        }

        if rest(&self.text_runs(page_index)?) != untouched {
            let restored = Self::open_bytes(snapshot, None)?;
            self.document = restored.document;
            self.page_count = restored.page_count;
            if let Ok(mut cached) = self.vault.lock() {
                *cached = None;
            }
            return Err(PdfError::Unsupported(
                "turning these words rewrites the rest of the page here, so nothing was changed; \
                 these words cannot be turned on this page",
            ));
        }
        self.touch();
        Ok(())
    }
}

/// What the old repair wrote where it should not have.
const MISPLACED_TYPE: &[u8] = b"/Type/XRef";

/// Take [`MISPLACED_TYPE`] out of the table at `stray` and put it in the
/// dictionary at `open`, both found by [`locate_misplaced_xref_type`]. The file
/// keeps its length: ten bytes out, ten bytes in.
fn move_xref_type(bytes: &mut Vec<u8>, open: usize, stray: usize) {
    bytes.drain(stray..stray + MISPLACED_TYPE.len());
    bytes.splice(open..open, MISPLACED_TYPE.iter().copied());
}

/// The file with the damage taken out, or `None` when it is not damaged that way.
fn repair_misplaced_xref_type(bytes: &[u8]) -> Option<Vec<u8>> {
    let (open, stray) = locate_misplaced_xref_type(bytes, 0)?;
    let mut mended = bytes.to_vec();
    move_xref_type(&mut mended, open, stray);
    Some(mended)
}

/// Mend a file whose last cross-reference stream an earlier build damaged.
///
/// **What was wrong, and why the fix did not mend files.** Before 0.1.36 the
/// repair `close_trailing_xref_object` makes after an incremental save went to
/// the wrong `<<` in some files: one inside the stream's binary data, which a
/// table of offsets can hold by chance. It wrote `/Type/XRef` there — ten bytes
/// more than the stream's `/Length` says — and left the dictionary without the
/// key a cross-reference stream must have. PDFium cannot read such a file
/// (`PdfiumLibraryInternalError(Unknown)`), and nothing but Pagify's own save
/// could have made one, so the files are real and they are the owner's.
///
/// What this does is exactly the reverse: take the ten bytes out of the data
/// and put them in the dictionary. The two cancel, so the file's length and
/// every offset in it — `startxref` included — stay as they were.
///
/// Where that damage is, as two places in `bytes`: just after the `<<` that
/// opens the cross-reference stream's dictionary, and the start of the stray
/// type in its table. `bytes` may be only the end of a file; `base` is where in
/// the file it begins, because `startxref` names an offset in the whole file.
///
/// `None` for any file that does not look like that damage precisely: no
/// cross-reference stream, a `/Length` that is not a plain number, a data
/// section not exactly ten bytes over (give or take the end of line), no
/// `/Type/XRef` in it, or a dictionary that already has a `/Type`. An undamaged
/// file, or one damaged some other way, is left to report its own error.
fn locate_misplaced_xref_type(bytes: &[u8], base: usize) -> Option<(usize, usize)> {
    let dict = trailing_dictionary_start_at(bytes, base)?;
    let startxref = find_last(bytes, b"startxref")?;
    let stream = dict + bytes.get(dict..startxref)?.windows(6).position(|w| w == b"stream")?;
    let dictionary = &bytes[dict..stream];
    if contains(dictionary, b"/Type") {
        return None;
    }
    let declared = declared_length(dictionary)?;

    let mut data = stream + b"stream".len();
    match bytes.get(data..) {
        Some([b'\r', b'\n', ..]) => data += 2,
        Some([b'\n', ..]) | Some([b'\r', ..]) => data += 1,
        _ => {}
    }
    let end = find_last(&bytes[..startxref], b"endstream")?;
    let region = bytes.get(data..end)?;
    // The end of line before `endstream` is not counted in `/Length`: up to two bytes.
    let extra = region.len().checked_sub(declared)?;
    if !(MISPLACED_TYPE.len()..=MISPLACED_TYPE.len() + 2).contains(&extra) {
        return None;
    }
    let stray = data + region.windows(MISPLACED_TYPE.len()).position(|w| w == MISPLACED_TYPE)?;
    Some((dict, stray))
}

/// The number after `/Length` in a stream's dictionary, when it is written
/// directly — not as a reference (`12 0 R`), which would need a lookup.
fn declared_length(dictionary: &[u8]) -> Option<usize> {
    let at = dictionary.windows(7).position(|w| w == b"/Length")? + 7;
    let rest = &dictionary[at..];
    let rest = &rest[rest.iter().take_while(|b| b.is_ascii_whitespace()).count()..];
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let number: usize = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    // `/Length 12 0 R` is a reference, and 12 is not the length.
    let after = &rest[digits..];
    let after = &after[after.iter().take_while(|b| b.is_ascii_whitespace()).count()..];
    let reference = after.first().is_some_and(u8::is_ascii_digit) && {
        let spaced = after.iter().skip_while(|b| b.is_ascii_digit()).skip_while(|b| b.is_ascii_whitespace());
        spaced.clone().next() == Some(&b'R')
    };
    (!reference).then_some(number)
}

/// Byte just after the `<<` opening the trailing cross-reference stream's
/// dictionary, if there is one.
///
/// Inserting there is offset-safe for the same reason closing the object is: the
/// only offset naming this object is `startxref`, which points at its *header* —
/// before the insertion — and every offset the table itself holds points at
/// objects earlier in the file. Nothing that is pointed at moves.
fn trailing_dictionary_start(bytes: &[u8]) -> Option<usize> {
    trailing_dictionary_start_at(bytes, 0)
}

/// As [`trailing_dictionary_start`] for `bytes` that are the file from `base`
/// on — the end of a large file, read without the rest. `startxref` names an
/// offset in the whole file, so it is brought into `bytes` first; an object
/// that lies before `bytes` begins is not found.
fn trailing_dictionary_start_at(bytes: &[u8], base: usize) -> Option<usize> {
    // **Found by following `startxref` to the object, never by searching
    // backwards for `<<`.** Reported from use: a saved file that would not open
    // again ("pdfium error: PdfiumLibraryInternalError(Unknown)"). The search
    // that used to be here started from the last `stream` before `startxref` —
    // which is the one inside `endstream`, at the *end* of the cross-reference
    // table's binary data — and took the nearest `<<` before it. A table of
    // four-byte offsets is a few thousand bytes of arbitrary numbers, and two
    // `<` bytes in a row turn up in it by chance (the offsets 0x3C3C00.. — a
    // file of about four megabytes on), so `/Type/XRef` was written INTO the
    // table: ten bytes more than its `/Length` said, and the file no longer
    // opened. Whether a save did that depended on the numbers in the file.
    let startxref = find_last(bytes, b"startxref")?;
    let digits: Vec<u8> = bytes[startxref + b"startxref".len()..]
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(u8::is_ascii_digit)
        .collect();
    let offset = std::str::from_utf8(&digits).ok()?.parse::<usize>().ok()?.checked_sub(base)?;
    // `169 0 obj <<`: the dictionary opens within a few bytes of the header.
    let header = bytes.get(offset..)?;
    let header = &header[..header.len().min(64)];
    let open = header.windows(2).position(|w| w == b"<<")?;
    Some(offset + open + 2)
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .rev()
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find_last(haystack, needle).is_some()
}

#[cfg(test)]
mod trailing_object_tests {
    use super::{close_trailing_xref_object, find_last};

    /// The exact shape PDFium produced on a real document.
    ///
    /// `startxref` names offset 0, where the object begins, as it does in a real
    /// cross-reference stream. That is not decoration: the repair follows the
    /// offset to decide whether the trailing cross-reference is a stream at all,
    /// so a fixture pointing nowhere would be declined — correctly — and would
    /// prove nothing.
    #[test]
    fn an_unclosed_xref_stream_is_closed() {
        let broken = b"1 0 obj\n<<>>stream\nxx\nendstream\nstartxref\n0\n%%EOF\n".to_vec();
        let fixed = close_trailing_xref_object(broken);
        let text = String::from_utf8_lossy(&fixed);

        assert!(text.contains("endstream\nendobj\nstartxref"), "got {text}");
    }

    #[test]
    fn a_classic_table_in_a_file_that_has_streams_is_left_alone() {
        // The regression, and the one that mattered: a document with a classic
        // table still contains content streams. Keying on "is there an endstream
        // somewhere?" found one belonging to an ordinary object in the middle of
        // the file and inserted there, pushing the table ten bytes past the offset
        // `startxref` names — turning a valid save into a damaged one.
        //
        // The `xref` keyword sits at offset 32 here, which is what `startxref`
        // says, so nothing may be inserted before it.
        let classic = b"1 0 obj
<<>>stream
xx
endstream
xref
0 1
trailer
<<>>
startxref
32
%%EOF
"
        .to_vec();
        assert_eq!(
            b"xref",
            &classic[32..36],
            "the fixture's own offset is wrong"
        );
        assert_eq!(classic.clone(), close_trailing_xref_object(classic));
    }

    #[test]
    fn a_classic_xref_table_is_left_alone() {
        // No stream at the end at all — the small fixtures, and every file that
        // saved cleanly before this was found.
        let classic =
            b"xref\n0 1\n0000000000 65535 f \ntrailer\n<<>>\nstartxref\n9\n%%EOF\n".to_vec();
        assert_eq!(classic.clone(), close_trailing_xref_object(classic));
    }

    #[test]
    fn an_xref_stream_missing_its_type_gains_one() {
        // The defect that actually stops a reader finding the table: PDFium omits
        // /Type /XRef, so startxref names an object nothing will accept as a
        // cross-reference stream.
        let broken = b"1 0 obj
<</Size 5/Prev 9>>stream
xx
endstream
endobj
startxref
0
%%EOF
"
        .to_vec();
        let fixed = close_trailing_xref_object(broken);
        let text = String::from_utf8_lossy(&fixed);

        assert!(text.contains("/Type/XRef"), "got {text}");
        assert!(
            text.contains("/Size 5"),
            "the rest of the dictionary survives: {text}"
        );
    }

    /// **Reported from use: a document that could not be opened again after a
    /// save.** The table's binary data holds two `<` bytes in a row, as a table
    /// of offsets sometimes does; the repair put `/Type/XRef` there, in the data,
    /// instead of in the dictionary. The data must come through byte for byte,
    /// the type must land in the dictionary, and `/Length` must still be true.
    // ---- mending a file the old repair already damaged ------------------------

    /// A cross-reference stream as PDFium writes it after an incremental save:
    /// no `/Type`, `\r\n` after `stream`, and the table's binary data holding
    /// `<<`. `damaged` is what the repair before 0.1.36 made of it: the type
    /// written into the data, after that `<<`.
    fn xref_file(data: &[u8], damaged: bool) -> Vec<u8> {
        let mut data = data.to_vec();
        let mut dictionary = format!("<</Info 3 0 R/Size 170/W[0 4 1]/Length {}", data.len());
        if damaged {
            let at = data.windows(2).position(|w| w == b"<<").expect("the data holds <<") + 2;
            data.splice(at..at, b"/Type/XRef".iter().copied());
        }
        dictionary.push_str(">>");
        let mut file = b"%PDF-1.5\n".to_vec();
        let object = file.len();
        file.extend_from_slice(b"169 0 obj ");
        file.extend_from_slice(dictionary.as_bytes());
        file.extend_from_slice(b"stream\r\n");
        file.extend_from_slice(&data);
        file.extend_from_slice(b"\r\nendstream\nendobj\nstartxref\n");
        file.extend_from_slice(object.to_string().as_bytes());
        file.extend_from_slice(b"\n%%EOF\n");
        file
    }

    const TABLE: &[u8] = &[0, 0, 0x12, 0x34, 1, 0, 0x3C, 0x3C, 0x00, 1, 0, 0, 0x55, 0x66, 1];

    #[test]
    fn a_file_with_the_type_written_into_its_table_is_mended_to_what_it_should_have_been() {
        let damaged = xref_file(TABLE, true);
        let mended = super::repair_misplaced_xref_type(&damaged).expect("the damage was not recognised");

        // Same length: the ten bytes taken out of the table are the ten put in
        // the dictionary, so every offset in the file stands.
        assert_eq!(mended.len(), damaged.len());
        // The table is back to exactly what it was...
        assert!(mended.windows(TABLE.len()).any(|w| w == TABLE), "the table was not restored");
        // ...and the key is where a reader looks for it, once.
        let text = String::from_utf8_lossy(&mended).into_owned();
        assert!(text.contains("169 0 obj <</Type/XRef/Info"), "{text}");
        assert_eq!(text.matches("/Type/XRef").count(), 1);
        // And it is now what the sound repair would have written, so it is left alone after.
        assert_eq!(super::close_trailing_xref_object(mended.clone()), mended);
    }

    /// Against a real pair: a document an earlier build damaged and the same
    /// document mended by hand, byte by byte, with a different tool. Set
    /// `PAGIFY_REAL_DAMAGED_PDF` and `PAGIFY_REAL_MENDED_PDF`.
    #[test]
    #[ignore = "needs a real damaged document and its hand-mended twin"]
    fn the_mending_matches_a_file_mended_by_hand() {
        let (Ok(damaged), Ok(mended)) =
            (std::env::var("PAGIFY_REAL_DAMAGED_PDF"), std::env::var("PAGIFY_REAL_MENDED_PDF"))
        else {
            return;
        };
        let (damaged, mended) = (std::fs::read(damaged).unwrap(), std::fs::read(mended).unwrap());
        let ours = super::repair_misplaced_xref_type(&damaged).expect("the damage was not recognised");
        assert_eq!(ours.len(), mended.len());
        assert!(ours == mended, "the two mended files differ");
    }

    /// A catalogue is 80 MB and the stream is the last object in it: asking
    /// whether it is damaged must not need the rest.
    #[test]
    fn the_damage_is_found_from_the_end_of_a_file_alone() {
        let damaged = xref_file(TABLE, true);
        let whole = super::locate_misplaced_xref_type(&damaged, 0).expect("not found in the whole file");

        let object = damaged.windows(9).position(|w| w == b"169 0 obj").unwrap();
        let from = object - 4;
        let (open, stray) = super::locate_misplaced_xref_type(&damaged[from..], from).expect("not found from the end");
        assert_eq!((open + from, stray + from), whole);

        // A tail that begins after the object cannot see it, and says nothing.
        assert_eq!(super::locate_misplaced_xref_type(&damaged[object + 1..], object + 1), None);
    }

    #[test]
    fn a_file_that_is_not_damaged_that_way_is_never_touched() {
        // Undamaged: PDFium's own shape, no `/Type`, a data section as long as `/Length` says.
        assert_eq!(super::repair_misplaced_xref_type(&xref_file(TABLE, false)), None);
        // Already mended: the dictionary has its type.
        let mended = super::repair_misplaced_xref_type(&xref_file(TABLE, true)).unwrap();
        assert_eq!(super::repair_misplaced_xref_type(&mended), None);
        // A `/Length` that is a reference says nothing about the data.
        let mut indirect = xref_file(TABLE, true);
        let text = String::from_utf8_lossy(&indirect).into_owned();
        let changed = text.replace(&format!("/Length {}", TABLE.len()), "/Length 12 0 R");
        indirect = changed.into_bytes();
        assert_eq!(super::repair_misplaced_xref_type(&indirect), None);
        // Not a file with a cross-reference stream at all.
        assert_eq!(super::repair_misplaced_xref_type(b"%PDF-1.4\nnot much of one\n"), None);
        assert_eq!(super::repair_misplaced_xref_type(b""), None);
    }

    #[test]
    fn the_declared_length_is_read_only_when_it_is_a_plain_number() {
        assert_eq!(super::declared_length(b"/Size 3/Length 25"), Some(25));
        assert_eq!(super::declared_length(b"/Length   7 /W[1 2 1]"), Some(7));
        assert_eq!(super::declared_length(b"/Length 25 0 R"), None);
        assert_eq!(super::declared_length(b"/Length"), None);
        assert_eq!(super::declared_length(b"/Size 3"), None);
    }

    #[test]
    fn a_table_whose_data_contains_two_angle_brackets_is_not_written_into() {
        // Four-byte offsets, as a real table has them: ...3C 3C... in the middle,
        // and the bytes "stream" and "/Type" appearing in it for good measure.
        let data: Vec<u8> = [&[0u8, 0, 0x12, 0x34, 1][..], &[0, 0x3C, 0x3C, 0x00, 1], b"/Type\0stream\0", &[0, 0, 0x55, 0x66, 1]].concat();
        let header = b"169 0 obj <</Info 3 0 R/Size 170/Prev 9/W[0 4 1]/Length ";
        let mut file = Vec::new();
        file.extend_from_slice(b"%PDF-1.7\n");
        let obj = file.len();
        file.extend_from_slice(header);
        file.extend_from_slice(data.len().to_string().as_bytes());
        file.extend_from_slice(b">>stream\r\n");
        file.extend_from_slice(&data);
        file.extend_from_slice(b"\r\nendstream\nstartxref\r\n");
        file.extend_from_slice(obj.to_string().as_bytes());
        file.extend_from_slice(b"\r\n%%EOF\r\n");

        let fixed = close_trailing_xref_object(file);
        let text = String::from_utf8_lossy(&fixed).into_owned();

        // The table's bytes are exactly what they were, and `/Length` still
        // names exactly them.
        let data_at = fixed.windows(data.len()).position(|w| w == &data[..]).expect("the table's data was altered");
        assert_eq!(&fixed[data_at - 2..data_at], b"\r\n");
        assert!(text.contains(&format!("/Length {}>>stream", data.len())), "{text}");
        // The type went where it belongs.
        let at = text.find("/Type/XRef").expect("no /Type/XRef at all");
        assert!(at < data_at, "/Type/XRef was written after the dictionary, at {at}: {text}");
        assert_eq!(text.matches("/Type/XRef").count(), 1);
    }

    #[test]
    fn a_dictionary_that_already_declares_its_type_is_not_given_a_second_one() {
        let good = b"1 0 obj
<</Type/XRef/Size 5>>stream
xx
endstream
endobj
startxref
0
%%EOF
"
        .to_vec();
        let fixed = close_trailing_xref_object(good.clone());

        assert_eq!(good, fixed);
    }

    #[test]
    fn a_file_with_no_trailer_is_returned_unchanged() {
        let odd = b"not a pdf at all".to_vec();
        assert_eq!(odd.clone(), close_trailing_xref_object(odd));
    }

    #[test]
    fn the_object_header_startxref_names_does_not_move() {
        // The one way these repairs could be worse than the bug they fix. Both
        // insert bytes *inside* the trailing object, so the offset that must not
        // shift is the one naming its header — which `startxref` holds, and which
        // every reader follows to find the table at all. Everything the table
        // itself points at lies earlier still.
        let broken =
            b"%PDF-1.7\n1 0 obj\n<</Size 5>>stream\nxx\nendstream\nstartxref\n9\n%%EOF\n".to_vec();
        let header = find_last(&broken, b"1 0 obj").expect("header");

        let fixed = close_trailing_xref_object(broken.clone());

        assert_eq!(
            header,
            find_last(&fixed, b"1 0 obj").expect("header"),
            "the header startxref points at moved",
        );
        assert_eq!(broken[..header], fixed[..header], "bytes before it changed");
    }
}

/// Whether the cross-reference `startxref` names is a stream rather than a table.
///
/// Read from the file's own declaration: the offset is parsed and the bytes there
/// are examined. A classic table begins with the `xref` keyword; anything else is
/// an object, which for a valid trailer means a cross-reference stream.
///
/// Conservative on every doubt — an offset that will not parse, or points past
/// the end, or names something unrecognisable — because the caller's next act is
/// to insert bytes into the file. Declining to repair leaves a file that at worst
/// still has the defect; repairing the wrong file makes one.
fn trailing_xref_is_a_stream(bytes: &[u8], startxref: usize) -> bool {
    let after = &bytes[startxref + b"startxref".len()..];
    let digits: Vec<u8> = after
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(u8::is_ascii_digit)
        .collect();

    let Ok(offset) = std::str::from_utf8(&digits).unwrap_or("").parse::<usize>() else {
        return false;
    };
    let Some(target) = bytes.get(offset..) else {
        return false;
    };

    !target.starts_with(b"xref")
}

// ------------------------------------------------------------ text marks --

/// The tag every text object this app writes carries.
///
/// Text is page content, so it has no annotation index to find it by; this is
/// what stands in for one. Proved by `examples/text_mark_probe.rs`: the tag
/// survives a save and a reopen, our objects can be told apart from the
/// document's own, and removing one leaves the rest alone.
const TEXT_MARK_NAME: &str = "PagifyText";

/// The parameter naming the mark, so one caption can be found on its own.
const TEXT_MARK_ID: &str = "id";

/// The parameter holding the app's description of the mark.
const TEXT_MARK_RESTORE: &str = "restore";

/// The Pagify id on this object, if it is one of ours.
///
/// Safety: `object` must be a live page object.
unsafe fn text_mark_id(
    bindings: &dyn pdfium_render::prelude::PdfiumLibraryBindings,
    object: FPDF_PAGEOBJECT,
) -> Option<i32> {
    for slot in 0..bindings.FPDFPageObj_CountMarks(object) {
        let mark = bindings.FPDFPageObj_GetMark(object, slot as c_ulong);
        if mark.is_null() || !mark_is_ours(bindings, mark) {
            continue;
        }
        let mut value = 0;
        if bindings.FPDFPageObjMark_GetParamIntValue(mark, TEXT_MARK_ID, &mut value) != 0 {
            return Some(value);
        }
    }
    None
}

/// The restore blob on this object, if it carries one.
///
/// Safety: as above.
unsafe fn text_mark_restore_of(
    bindings: &dyn pdfium_render::prelude::PdfiumLibraryBindings,
    object: FPDF_PAGEOBJECT,
) -> Option<String> {
    for slot in 0..bindings.FPDFPageObj_CountMarks(object) {
        let mark = bindings.FPDFPageObj_GetMark(object, slot as c_ulong);
        if mark.is_null() || !mark_is_ours(bindings, mark) {
            continue;
        }

        let mut needed: c_ulong = 0;
        if bindings.FPDFPageObjMark_GetParamStringValue(
            mark,
            TEXT_MARK_RESTORE,
            std::ptr::null_mut(),
            0,
            &mut needed,
        ) == 0
            || needed == 0
        {
            continue;
        }

        let mut buffer = vec![0u16; needed as usize / 2 + 1];
        let mut written: c_ulong = 0;
        if bindings.FPDFPageObjMark_GetParamStringValue(
            mark,
            TEXT_MARK_RESTORE,
            buffer.as_mut_ptr(),
            needed,
            &mut written,
        ) == 0
        {
            continue;
        }

        // The length is bytes and counts the terminator.
        let characters = (written as usize / 2).saturating_sub(1);
        return Some(String::from_utf16_lossy(&buffer[..characters]));
    }
    None
}

/// Whether this mark is one of ours rather than the document's own.
///
/// Safety: `mark` must be a live content mark.
unsafe fn mark_is_ours(
    bindings: &dyn pdfium_render::prelude::PdfiumLibraryBindings,
    mark: FPDF_PAGEOBJECTMARK,
) -> bool {
    let mut buffer = [0u16; 64];
    let mut length: c_ulong = 0;
    if bindings.FPDFPageObjMark_GetName(
        mark,
        buffer.as_mut_ptr(),
        (buffer.len() * 2) as c_ulong,
        &mut length,
    ) == 0
    {
        return false;
    }
    // `length` is what the name *needs*, not what the buffer got: a name
    // longer than the buffer reports a length past its end, and slicing by it
    // collapsed the document on open. Found by audit. A name that does not
    // fit is not ours in any case.
    let characters = (length as usize / 2).saturating_sub(1).min(buffer.len());
    String::from_utf16_lossy(&buffer[..characters]) == TEXT_MARK_NAME
}

/// Embed a registered font in the document, ready to be written by glyph id.
///
/// Three things have to be right and each was found the hard way:
///
/// * `FPDFText_LoadCidType2Font`, not `FPDFText_LoadFont`. The simpler call
///   embeds the font perfectly and builds its own ToUnicode by running the
///   font's cmap backwards — and a joined form has no character to run back to.
///   The words drew correctly and came out of the file as `اϨʹ۰ՍЪة`:
///   unsearchable, uncopyable, and completely silent about it.
/// * a ToUnicode CMap of our own, built from the shaper's clusters, so the words
///   are still words afterwards.
/// * an explicit identity CID-to-glyph table. Passing none makes the call return
///   null with nothing said about why.
///
/// Safety: `document` must be live for the call.
unsafe fn load_embedded_font(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    name: &str,
    glyphs: &[Glyph],
) -> Result<(FPDF_FONT, Vec<u32>)> {
    // Cut the font down to the glyphs this caption uses. Embedded whole, a
    // four-character Chinese note put sixteen megabytes into the document.
    let wanted: Vec<u32> = glyphs.iter().map(|g| g.id).collect();
    let subset = crate::text::subset(name, &wanted)?;
    let cid_to_gid = crate::text::identity_table(subset.glyph_count);

    // Renumbered, so the ToUnicode is keyed by the ids that are actually in
    // the page. Built from the glyphs as written rather than by shaping again:
    // shaping twice is two chances to disagree.
    let renumbered: Vec<Glyph> = glyphs
        .iter()
        .zip(&subset.ids)
        .map(|(glyph, &id)| Glyph { id, ..glyph.clone() })
        .collect();
    let to_unicode = crate::text::to_unicode_from_glyphs(&renumbered);

    let font = bindings.FPDFText_LoadCidType2Font(
        document,
        subset.data.as_ptr(),
        subset.data.len() as c_uint,
        &to_unicode,
        cid_to_gid.as_ptr(),
        cid_to_gid.len() as c_uint,
    );
    if font.is_null() {
        return Err(PdfError::Pdfium(format!("{name} would not embed")));
    }
    Ok((font, subset.ids))
}

/// Moving pages between documents.
///
/// Both directions are `FPDF_ImportPagesByIndex`. What it takes *with* a page was
/// measured rather than assumed — `examples/page_transfer_probe.rs` builds a
/// fixture of three differently-sized pages, each with text and one of our
/// marked-content tags, and checks all three survive the round trip. They do.
/// That last one mattered: nothing in PDFium's contract promises a private tag
/// survives a cross-document import, and the tag is what makes saved words an
/// editable and erasable mark.
impl PdfiumDocument {
    /// The raw PDFium handle. See [`Document::backend_handle`].
    fn handle(&self) -> FPDF_DOCUMENT {
        self.document.handle()
    }

    fn import_pages_from(
        &mut self,
        source: FPDF_DOCUMENT,
        indices: &[usize],
        at: usize,
    ) -> Result<usize> {
        if at > self.page_count {
            return Err(PdfError::InvalidArgument(format!(
                "cannot insert at {at}: the document has {} pages",
                self.page_count,
            )));
        }

        let wanted: Vec<c_int> = indices
            .iter()
            .map(|&index| {
                c_int::try_from(index).map_err(|_| {
                    PdfError::InvalidArgument(format!("page index {index} is out of range"))
                })
            })
            .collect::<Result<_>>()?;

        let bindings = pdfium()?.bindings();
        let before = self.page_count;

        // Safety: both documents are live for the call, and the index list is
        // valid for the length given. A null pointer with length zero is
        // PDFium's own spelling of "every page".
        let ok = unsafe {
            bindings.FPDF_ImportPagesByIndex(
                self.handle(),
                source,
                if wanted.is_empty() {
                    std::ptr::null()
                } else {
                    wanted.as_ptr()
                },
                wanted.len() as c_ulong,
                c_int::try_from(at).map_err(|_| {
                    PdfError::InvalidArgument(format!("insert position {at} is out of range"))
                })?,
            )
        };
        if ok == 0 {
            return Err(PdfError::Pdfium("the pages were refused".into()));
        }

        // Counted from the document rather than from the request: with an empty
        // list the caller does not know how many arrived, and after a partial
        // failure neither would we.
        self.page_count = self.document.pages().len() as usize;
        self.touch();
        Ok(self.page_count.saturating_sub(before))
    }
}

#[cfg(test)]
mod owned_code_tests {
    use super::{align_codes, encodable, owned_codes, typeable};

    fn font(characters: &str) -> std::collections::BTreeMap<String, u32> {
        characters
            .chars()
            .enumerate()
            .map(|(at, c)| (c.to_string(), at as u32 + 1))
            .collect()
    }

    /// **A run must always be retypable with its own words.**
    ///
    /// PDFium puts a space on the end of a run's text; a font that draws spaces
    /// by moving the pen has no glyph for one. Refusing on that made four runs
    /// of twenty-nine on a real document impossible to retype unchanged.
    #[test]
    fn a_trailing_space_no_glyph_exists_for_is_dropped_rather_than_refused() {
        let have = font("Componet");
        assert_eq!(encodable("Component ", &have), "Component");
        assert_eq!(encodable("Component\n\t ", &have), "Component");
    }

    /// But only the tail, and only whitespace.
    #[test]
    fn a_space_in_the_middle_of_the_words_is_left_alone() {
        let have = font("abc");
        // Refused later by the encoder, which is right: closing the gap would
        // change the words rather than tidy them.
        assert_eq!(encodable("a b", &have), "a b");
        // A space the font *does* have is kept wherever it is.
        let with_space = font("abc ");
        assert_eq!(encodable("abc ", &with_space), "abc ");
    }

    #[test]
    fn a_refusal_says_what_can_be_typed_instead() {
        assert_eq!(typeable(&font("cba")), "abc");
        // Whitespace is not something to offer somebody as a character.
        assert_eq!(typeable(&font("ab ")), "ab");
        assert_eq!(typeable(&font(" ")), "nothing this can read");
        // A whole alphabet is a wall, not an answer.
        let many: String = (33u8..=126).map(char::from).collect();
        assert!(typeable(&font(&many)).contains("different characters"));
    }

    /// **The bug that put a page footer on the page twice.**
    ///
    /// A run whose extracted text ends in a space PDFium invented: the last
    /// entry of the alignment is the sentinel, and taking it as a code index
    /// made the replaced range `0..usize::MAX + 1` — which wraps, in a release
    /// build, to the empty range. The replacement was written and nothing was
    /// taken out.
    #[test]
    fn a_trailing_space_pdfium_invented_does_not_swallow_the_range() {
        // Three codes spelling "abc", plus a fourth character no code produced.
        let owner = vec![0, 1, 2, usize::MAX];
        assert_eq!(owned_codes(&owner), Some(0..3), "the sentinel was read as a code");
        // The shape that actually wrapped: it must never come back empty.
        let range = owned_codes(&owner).expect("a range");
        assert!(!range.is_empty(), "an empty range replaces nothing and inserts everything");
    }

    /// The ordinary case is unchanged.
    #[test]
    fn a_run_that_owns_every_code_covers_all_of_them() {
        assert_eq!(owned_codes(&[0, 1, 2, 3]), Some(0..4));
        // A ligature: one code spelling two characters.
        assert_eq!(owned_codes(&[0, 1, 1, 2]), Some(0..3));
        // A run starting part-way into an operator.
        assert_eq!(owned_codes(&[4, 5, 6]), Some(4..7));
    }

    #[test]
    fn several_invented_characters_are_all_ignored() {
        assert_eq!(owned_codes(&[usize::MAX, 0, 1, usize::MAX, usize::MAX]), Some(0..2));
    }

    /// Nothing owned by any code is refused rather than guessed at.
    #[test]
    fn characters_belonging_to_no_code_at_all_have_no_range() {
        assert_eq!(owned_codes(&[]), None);
        assert_eq!(owned_codes(&[usize::MAX, usize::MAX]), None);
    }

    /// **End to end, from the shape the catalogue actually had.**
    ///
    /// The font's table spells the words; PDFium reports them with one space
    /// more at the end than any code accounts for.
    #[test]
    fn the_reported_case_lines_up_and_covers_every_code() {
        let spellings: Vec<Option<String>> = "HUE . SATURATION . INTENSITY"
            .chars()
            .map(|c| Some(c.to_string()))
            .collect();
        let codes = spellings.len();

        let owner = align_codes(&spellings, "HUE . SATURATION . INTENSITY ")
            .expect("the trailing space is not supposed to break the alignment");
        assert_eq!(*owner.last().expect("an entry"), usize::MAX, "control");

        let range = owned_codes(&owner).expect("a range");
        assert_eq!(range, 0..codes, "the replacement would have missed a code");
    }
}

#[cfg(test)]
mod order_tests {
    use super::{nearest_placed, placed_in_order};
    use crate::document::{Color, Point, Rect, TextRun};
    use crate::pdf::content::{Origin, Placed};

    const HEIGHT: f32 = 800.0;

    /// A run whose box sits at `(x, y)` in top-left coordinates.
    fn run(object: usize, x: f32, y: f32) -> TextRun {
        TextRun {
            object,
            text: "words".into(),
            rect: Rect { left: x, top: y - 8.0, right: x + 40.0, bottom: y },
            origin: Point { x, y: y - 1.0 },
            size: 10.0,
            color: Color { r: 0, g: 0, b: 0, a: 255 },
        }
    }

    /// An operator drawing at `(x, y)` in top-left coordinates, so it lines up
    /// with the run above.
    fn placed(at: usize, x: f32, y: f32) -> Placed {
        Placed {
            origin: Origin { operation: at, x, y: HEIGHT - y },
            font: Some(b"F1".to_vec()),
            size: 10.0,
            line: at,
            scale: 1.0,
            axis: (1.0, 0.0),
        }
    }

    /// **The measured case.** Some runs place by position; the rest are
    /// continuations whose origin the stream walk could not advance, and they
    /// are resolved by counting.
    #[test]
    fn a_run_the_walk_could_not_place_is_found_by_counting() {
        let runs = vec![run(0, 100.0, 100.0), run(1, 200.0, 100.0), run(2, 100.0, 200.0)];
        // The middle operator is reported back at the first one's origin, which
        // is what an unadvanced text matrix looks like.
        let ops = vec![placed(0, 100.0, 100.0), placed(1, 100.0, 100.0), placed(2, 100.0, 200.0)];

        assert!(nearest_placed(&ops, 200.0, HEIGHT - 100.0).is_none(), "control");
        let found = placed_in_order(&runs, &ops, HEIGHT, 1).expect("counted");
        assert_eq!(found.origin.operation, 1, "it counted to the wrong operator");
    }

    /// **A page that has not proved anything gets no benefit of the doubt.**
    #[test]
    fn counting_is_refused_where_the_order_is_not_the_order() {
        let runs = vec![run(0, 100.0, 100.0), run(1, 200.0, 100.0), run(2, 100.0, 200.0)];
        // The first run's operator is the second one: the orders differ, so
        // counting would edit the wrong words.
        let ops = vec![placed(0, 100.0, 200.0), placed(1, 100.0, 100.0), placed(2, 900.0, 900.0)];
        assert!(placed_in_order(&runs, &ops, HEIGHT, 2).is_none());
    }

    #[test]
    fn counting_is_refused_when_the_counts_do_not_match() {
        let runs = vec![run(0, 100.0, 100.0), run(1, 200.0, 100.0)];
        let ops = vec![placed(0, 100.0, 100.0)];
        assert!(placed_in_order(&runs, &ops, HEIGHT, 1).is_none());
    }

    /// **Nothing placed is not evidence of anything.** A page where the walk
    /// lost every origin must not be edited by counting alone.
    #[test]
    fn counting_is_refused_without_enough_confirmations() {
        let runs = vec![run(0, 100.0, 100.0), run(1, 200.0, 100.0), run(2, 100.0, 200.0)];
        let ops = vec![placed(0, 500.0, 500.0), placed(1, 600.0, 500.0), placed(2, 700.0, 500.0)];
        assert!(placed_in_order(&runs, &ops, HEIGHT, 0).is_none());

        // And one confirmation out of three is still not half.
        let ops = vec![placed(0, 100.0, 100.0), placed(1, 600.0, 500.0), placed(2, 700.0, 500.0)];
        assert!(placed_in_order(&runs, &ops, HEIGHT, 2).is_none());
    }

    #[test]
    fn an_index_past_the_end_is_not_a_match() {
        let runs = vec![run(0, 100.0, 100.0)];
        let ops = vec![placed(0, 100.0, 100.0)];
        assert!(placed_in_order(&runs, &ops, HEIGHT, 7).is_none());
    }

    // -- the operator found by count is checked against the stream -------------

    /// A run's origin is `(x, y - 1)` in these tests, in top-left coordinates —
    /// `HEIGHT - that` in the stream's own — and its text is `words` (five
    /// characters).
    fn agrees(operator: &Placed, run_x: f32, run_y: f32, codes: usize) -> bool {
        let mut r = run(0, run_x, run_y);
        r.text = "words".into();
        super::order_agrees(&r, operator, r.origin.x, HEIGHT - r.origin.y, &|_| codes)
    }

    /// The run starts where the operator's own positioning put the pen.
    #[test]
    fn an_operator_at_the_runs_own_origin_with_the_right_length_agrees() {
        assert!(agrees(&placed(0, 100.0, 100.0), 100.0, 100.0, 5));
    }

    /// Or further along the same line — a continuation is drawn after the pen
    /// advanced, and the walk has no way to know by how much.
    #[test]
    fn a_continuation_further_along_the_same_line_agrees() {
        assert!(agrees(&placed(0, 100.0, 100.0), 160.0, 100.0, 5));
    }

    /// Never another line, and never behind the pen.
    #[test]
    fn another_line_or_a_place_behind_the_operator_does_not() {
        assert!(!agrees(&placed(0, 100.0, 100.0), 160.0, 112.0, 5), "a line below");
        assert!(!agrees(&placed(0, 100.0, 100.0), 90.0, 100.0, 5), "behind the pen");
    }

    /// An operator that draws *fewer* codes than the run has characters is
    /// another operator — less the one PDFium sometimes appends. More is fine:
    /// a run of spaces is one in PDFium's text and however many the font drew.
    #[test]
    fn an_operator_drawing_too_little_for_the_run_does_not() {
        let at = placed(0, 100.0, 100.0);
        assert!(agrees(&at, 100.0, 100.0, 5), "exactly as many");
        assert!(agrees(&at, 100.0, 100.0, 4), "one less: the space PDFium adds");
        assert!(agrees(&at, 100.0, 100.0, 13), "more: collapsed spaces");
        assert!(!agrees(&at, 100.0, 100.0, 3), "two less is another operator");
        assert!(!agrees(&at, 100.0, 100.0, 1));
    }

    /// A run whose font has no character map reads as nothing, and cannot be
    /// held to a length it has no way to state.
    #[test]
    fn a_run_that_reads_as_nothing_is_not_held_to_a_length() {
        let at = placed(0, 100.0, 100.0);
        let mut r = run(0, 100.0, 100.0);
        r.text = String::new();
        assert!(super::order_agrees(&r, &at, r.origin.x, HEIGHT - r.origin.y, &|_| 9));
    }

    /// Text set at an angle runs along its own axis, not the page's.
    #[test]
    fn along_is_along_the_way_the_text_runs() {
        let mut up = placed(0, 100.0, 100.0);
        up.axis = (0.0, 1.0);
        // Text running up the page: a continuation is further up (larger y in
        // the stream's own coordinates, smaller in the run's top-left ones).
        assert!(agrees(&up, 100.0, 40.0, 5), "50 points along");
        assert!(!agrees(&up, 160.0, 100.0, 5), "across the way it runs");
    }

    // -- the lookup the edits use: by count, held to the stream ----------------

    /// The operation the `which`th of `total` objects is found at by count, for
    /// operators that all draw `codes` codes.
    fn by_count(run: &TextRun, ops: &[Placed], order: Option<(usize, usize)>, codes: usize) -> Option<usize> {
        super::placed_by_order(run, HEIGHT, ops, order, &|_| codes).map(|p| p.origin.operation)
    }

    /// **The case position cannot do.** The second piece of a line is drawn after
    /// the first, 60 pt along it; the walk reports it at the first one's origin,
    /// where nothing of it is. Counting finds it.
    #[test]
    fn a_continuation_is_found_by_count_where_position_finds_nothing() {
        let ops = vec![placed(0, 100.0, 100.0), placed(1, 100.0, 100.0)];
        let second = run(1, 160.0, 100.0);
        assert!(nearest_placed(&ops, second.origin.x, HEIGHT - second.origin.y).is_none(), "control");
        assert_eq!(by_count(&second, &ops, Some((1, 2)), 5), Some(1));
    }

    /// Only where the page has as many operators as text objects.
    #[test]
    fn no_count_is_taken_where_the_totals_differ() {
        let ops = vec![placed(0, 100.0, 100.0), placed(1, 100.0, 100.0)];
        let second = run(1, 160.0, 100.0);
        assert_eq!(by_count(&second, &ops, Some((1, 3)), 5), None, "an object more than there are operators");
        assert_eq!(by_count(&second, &ops, Some((1, 1)), 5), None, "an operator more than there are objects");
    }

    /// An object that is not a text object has no place in the order.
    #[test]
    fn an_object_with_no_place_in_the_order_is_not_counted() {
        let ops = vec![placed(0, 100.0, 100.0), placed(1, 100.0, 100.0)];
        assert_eq!(by_count(&run(1, 160.0, 100.0), &ops, None, 5), None);
        assert_eq!(by_count(&run(1, 160.0, 100.0), &ops, Some((2, 2)), 5), None, "past the last operator");
    }

    /// **The stream has the last word.** The right count and the wrong operator —
    /// on another line, or behind the pen, or too short for the run — is refused,
    /// and the caller falls back to the position match.
    #[test]
    fn a_count_the_stream_does_not_bear_out_is_refused() {
        let line_below = vec![placed(0, 100.0, 100.0), placed(1, 100.0, 140.0)];
        assert_eq!(by_count(&run(1, 160.0, 100.0), &line_below, Some((1, 2)), 5), None, "another line");
        let behind = vec![placed(0, 100.0, 100.0), placed(1, 300.0, 100.0)];
        assert_eq!(by_count(&run(1, 160.0, 100.0), &behind, Some((1, 2)), 5), None, "behind the pen");
        let ops = vec![placed(0, 100.0, 100.0), placed(1, 100.0, 100.0)];
        assert_eq!(by_count(&run(1, 160.0, 100.0), &ops, Some((1, 2)), 2), None, "too short for the run");
    }
}

#[cfg(test)]
mod fill_tests {
    use super::{drawn_in_colour, fill_before};
    use crate::document::Color;
    use crate::pdf::content;

    /// What puts the fill colour back after the first show-text operator in
    /// `stream`.
    fn restored_after_the_show(stream: &str) -> String {
        let bytes = stream.as_bytes();
        let operations = content::parse(bytes).expect("parse");
        let at = operations.iter().position(|o| o.shows_text()).expect("a show-text operator");
        String::from_utf8(fill_before(bytes, &operations, at)).expect("utf8")
    }

    /// Nothing set a colour: the page's own initial one.
    #[test]
    fn a_page_that_never_set_a_colour_gets_black_in_device_gray() {
        assert_eq!(restored_after_the_show("BT /F1 12 Tf (a) Tj ET"), "0 g");
    }

    /// The nearest device colour is the whole of the state, whatever came before.
    #[test]
    fn the_nearest_device_colour_is_all_of_it() {
        assert_eq!(restored_after_the_show("0.2 g 1 0 0 rg BT (a) Tj ET"), "1 0 0 rg");
        assert_eq!(restored_after_the_show("1 0 0 rg 0 0 0 1 k BT (a) Tj ET"), "0 0 0 1 k");
        assert_eq!(restored_after_the_show("/DeviceRGB cs 0.2 0.4 0.6 sc 0.5 g BT (a) Tj ET"), "0.5 g");
    }

    /// A colour space and the value set in it are two operators and both come back.
    #[test]
    fn a_colour_space_and_its_value_come_back_together() {
        assert_eq!(
            restored_after_the_show("/DeviceRGB cs 0.2 0.4 0.6 sc BT (a) Tj ET"),
            "/DeviceRGB cs\n0.2 0.4 0.6 sc"
        );
        assert_eq!(restored_after_the_show("0.5 g 0.3 sc BT (a) Tj ET"), "0.5 g\n0.3 sc");
        assert_eq!(restored_after_the_show("0.3 sc BT (a) Tj ET"), "0.3 sc", "a value in the initial space");
        // The value that is in force, not one it replaced.
        assert_eq!(
            restored_after_the_show("/DeviceRGB cs 0.1 0.1 0.1 sc 0.2 0.4 0.6 sc BT (a) Tj ET"),
            "/DeviceRGB cs\n0.2 0.4 0.6 sc"
        );
    }

    /// A block that closed before the operator is not in force; one still open is.
    #[test]
    fn a_block_that_closed_is_stepped_over_and_one_still_open_is_not() {
        assert_eq!(restored_after_the_show("0.5 g q 1 0 0 rg Q BT (a) Tj ET"), "0.5 g");
        assert_eq!(
            restored_after_the_show("0.5 g q 0.1 g q 0.2 g Q Q BT (a) Tj ET"),
            "0.5 g",
            "two blocks closed, one inside the other"
        );
        assert_eq!(restored_after_the_show("0.5 g q 0.1 g Q q 0.2 g BT (a) Tj ET Q"), "0.2 g", "the one still open");
        assert_eq!(restored_after_the_show("0.9 g q BT (a) Tj ET Q"), "0.9 g", "opened, and nothing set inside it");
    }

    /// Text is drawn in the fill colour; the stroke colour is another state.
    #[test]
    fn the_stroke_colour_is_not_the_fill() {
        assert_eq!(restored_after_the_show("0.2 g 1 0 0 RG 0 0 1 SC BT (a) Tj ET"), "0.2 g");
    }

    /// The operator, between the colour it is hidden in and the colour put back.
    #[test]
    fn the_operator_is_drawn_between_the_new_colour_and_the_old_one() {
        let stream = "0 0 0 1 k BT /F1 12 Tf (a) Tj (b) Tj ET";
        let operations = content::parse(stream.as_bytes()).expect("parse");
        let at = operations.iter().rposition(|o| o.shows_text()).expect("the second operator");
        let wrapped = drawn_in_colour(stream.as_bytes(), &operations, at, Color { r: 255, g: 51, b: 0, a: 255 });
        assert_eq!(String::from_utf8(wrapped).expect("utf8"), "1.000 0.200 0.000 rg\n(b) Tj\n0 0 0 1 k");
    }
}

#[cfg(test)]
mod line_removal_tests {
    use super::{carried_after_removal, draws_nothing, quote_effect, render_mode_before};
    use crate::document::{Color, Point, Rect, TextRun};
    use crate::pdf::content;
    use std::collections::HashSet;

    fn parse(stream: &str) -> Vec<content::Operation> {
        content::parse(stream.as_bytes()).expect("parse")
    }

    /// The `n`th show-text operation's index.
    fn show(operations: &[content::Operation], n: usize) -> usize {
        operations.iter().enumerate().filter(|(_, o)| o.shows_text()).nth(n).map(|(i, _)| i).expect("a show-text operator")
    }

    /// `Tj` and `TJ` leave nothing behind; the quote operators leave the line
    /// they move to, and `"` the spacing it sets.
    #[test]
    fn a_quote_operator_leaves_its_line_move_behind_and_the_others_nothing() {
        let operations = parse("BT (a) Tj [(b)] TJ (c) ' 3 1 (d) \" ET");
        let effects: Vec<String> = operations
            .iter()
            .filter(|o| o.shows_text())
            .map(|o| String::from_utf8(quote_effect(o).expect("an effect")).expect("utf8"))
            .collect();
        assert_eq!(effects, ["", "", "T*", "3 Tw\n1 Tc\nT*"]);
    }

    /// A `"` whose spacing is not a number cannot be written back.
    #[test]
    fn a_double_quote_operator_with_operands_that_are_not_numbers_is_refused() {
        let operations = parse("BT /a /b (d) \" ET");
        let quote = operations.iter().find(|o| o.shows_text()).expect("an operator");
        assert!(quote_effect(quote).is_err());
        let operations = parse("BT (d) \" ET");
        assert!(quote_effect(operations.iter().find(|o| o.shows_text()).expect("an operator")).is_err(), "too few operands");
    }

    /// The nearest `Tr` still in force; fill where none was set.
    #[test]
    fn the_rendering_mode_is_the_nearest_one_still_in_force() {
        let at = |stream: &str| {
            let operations = parse(stream);
            let index = show(&operations, 0);
            render_mode_before(&operations, index)
        };
        assert_eq!(at("BT (a) Tj ET"), 0);
        assert_eq!(at("7 Tr BT (a) Tj ET"), 7);
        assert_eq!(at("7 Tr 0 Tr BT (a) Tj ET"), 0);
        assert_eq!(at("7 Tr q 0 Tr Q BT (a) Tj ET"), 7, "a block that closed is not in force");
        assert_eq!(at("0 Tr q 5 Tr BT (a) Tj ET Q"), 5, "one still open is");
    }

    /// Cutting a piece out moves exactly the pieces the pen places after it.
    #[test]
    fn only_a_piece_the_pen_places_is_moved_by_what_comes_out_before_it() {
        let moved = |stream: &str, removing: &[usize]| {
            let operations = parse(stream);
            let removed: HashSet<usize> = removing.iter().map(|n| show(&operations, *n)).collect();
            let first = show(&operations, removing[0]);
            !carried_after_removal(&operations, first, &removed).is_empty()
        };
        assert!(moved("BT (a) Tj (b) Tj ET", &[0]), "the next piece starts where this one ended");
        assert!(moved("BT (a) Tj /F1 9 Tf 0 g (b) Tj ET", &[0]), "state operators do not place a piece");
        assert!(moved("BT (a) Tj (b) Tj (c) Tj ET", &[0, 1]), "the third still starts where the second ended");
        assert!(!moved("BT (a) Tj (b) Tj ET", &[0, 1]), "both go");
        assert!(!moved("BT (a) Tj 5 0 Td (b) Tj ET", &[0]), "a Td places the next piece");
        assert!(!moved("BT (a) Tj 1 0 0 1 5 5 Tm (b) Tj ET", &[0]), "a Tm does");
        assert!(!moved("BT (a) Tj T* (b) Tj ET", &[0]), "a T* does");
        assert!(!moved("BT (a) Tj (b) ' ET", &[0]), "a quote moves to the next line first");
        assert!(!moved("BT (a) Tj 1 2 (b) \" ET", &[0]), "so does the other");
        assert!(!moved("BT (a) Tj ET BT (b) Tj ET", &[0]), "a new text object starts afresh");
        assert!(!moved("BT (a) Tj ET", &[0]), "nothing after it at all");
    }

    /// What the pen carries on to is every piece that stays and has nothing
    /// repositioning it — including the ones after a piece that stays, which is
    /// how a line with a space between its words is seen: removing `(a)` carries
    /// the space, and the word after the space, and what follows that.
    #[test]
    fn what_the_pen_carries_on_to_is_every_kept_piece_before_the_next_placement() {
        let operations = parse("BT (a) Tj ( ) Tj (b) Tj ( ) Tj (c) Tj 5 0 Td (d) Tj ET");
        let shows: Vec<usize> = (0..6).map(|n| show(&operations, n)).collect();
        let removed: HashSet<usize> = [0, 2, 4].iter().map(|n| shows[*n]).collect();
        // Taking out a, b and c: the two spaces are carried; d is placed by its Td.
        assert_eq!(carried_after_removal(&operations, shows[0], &removed), [shows[1], shows[3]]);
        assert_eq!(carried_after_removal(&operations, shows[2], &removed), [shows[3]]);
        assert!(carried_after_removal(&operations, shows[4], &removed).is_empty());
    }

    fn run_in(rect: Rect, alpha: u8) -> TextRun {
        TextRun {
            object: 0,
            text: String::new(),
            rect,
            origin: Point { x: rect.left, y: rect.bottom },
            size: 8.0,
            color: Color { r: 0, g: 0, b: 0, a: alpha },
        }
    }

    /// A space has a box of no area at all — or the advance of a space and no
    /// height, as the space of a justified line does (measured 16.35 pt wide, 0
    /// tall, between two words of the datasheet); a hair-line letter has some.
    #[test]
    fn an_object_draws_nothing_when_it_has_no_area_or_is_not_painted() {
        let point = Rect { left: 357.4, top: 378.6, right: 357.4, bottom: 378.6 };
        let wide_space = Rect { left: 150.512, top: 536.201, right: 166.864, bottom: 536.201 };
        let stem = Rect { left: 10.0, top: 0.0, right: 10.4, bottom: 6.0 };
        let dash = Rect { left: 10.0, top: 3.0, right: 12.2, bottom: 3.4 };
        assert!(draws_nothing(&run_in(point, 255), 0), "a space: no area at all");
        assert!(draws_nothing(&run_in(wide_space, 255), 0), "a space with an advance and no height has no ink either");
        assert!(!draws_nothing(&run_in(stem, 255), 0), "a lone `l` is 0.4 pt wide and is there to be seen");
        assert!(!draws_nothing(&run_in(dash, 255), 0), "a hyphen is 0.4 pt tall and is there to be seen");
        assert!(draws_nothing(&run_in(stem, 0), 0), "fully transparent");
        assert!(draws_nothing(&run_in(stem, 255), 3), "invisible render mode");
        assert!(!draws_nothing(&run_in(stem, 255), 0));
    }
}

#[cfg(test)]
mod colour_key_tests {
    use super::*;

    /// Found by audit: `0é00000` is eight bytes and not eight digits.
    #[test]
    fn a_colour_key_that_is_not_hex_is_ignored_rather_than_sliced() {
        assert!(packed_colour("0é00000").is_none());
        assert!(packed_colour("0é000000").is_none());
        assert!(packed_colour("zz000000").is_none());
        assert!(packed_colour("ff00ff").is_none());
        assert_eq!(
            packed_colour("80FF7f00"),
            Some(Color { a: 0x80, r: 0xff, g: 0x7f, b: 0x00 })
        );
    }
}

#[cfg(test)]
mod vault_cache_tests {
    use super::*;

    /// **A lock that cannot be read is read once.** Found by audit: the
    /// failure fell through `?` before the cache was written, so a large,
    /// bad attachment was pulled out of the file and re-parsed every frame.
    #[test]
    fn a_lock_that_cannot_be_read_is_read_once_and_the_failure_kept() {
        if std::env::var_os("PAGIFY_PDFIUM_LIB").is_none() {
            eprintln!("skipping: PAGIFY_PDFIUM_LIB is not set");
            return;
        }
        crate::registry::exclusive(|| {
            let doc = PdfiumDocument::open_path(
                concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/bad-lock.pdf"),
                None,
            )
            .expect("open");
            let first = doc.read_vault().expect_err("a lock that is not one read as one");
            assert!(
                matches!(doc.vault.lock().expect("cache").as_ref(), Some(Err(_))),
                "the failure was not cached"
            );
            let started = std::time::Instant::now();
            let second = doc.read_vault().expect_err("read as one the second time");
            assert!(started.elapsed() < std::time::Duration::from_millis(5), "it read the file again");
            assert_eq!(first.to_string(), second.to_string());
        });
    }
}

#[cfg(test)]
mod page_tree_dag_tests {
    use super::*;

    /// A page tree whose every level names the same next node twice — a DAG,
    /// not a tree. Without a visited set, `depth` levels of that doubles the
    /// work `depth` times over: at `depth` = 20 that is over two million
    /// redundant resolves to reach one real page.
    fn dag_page_tree(depth: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");

        let total_objects = depth + 2; // catalog + `depth` Pages nodes + 1 Page
        let leaf_number = total_objects;

        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        for level in 0..depth {
            let number = 2 + level;
            let child = number + 1;
            offsets.push(out.len());
            out.extend_from_slice(
                format!(
                    "{number} 0 obj\n<< /Type /Pages /Kids [{child} 0 R {child} 0 R] /Count 1 >>\nendobj\n"
                )
                .as_bytes(),
            );
        }

        offsets.push(out.len());
        out.extend_from_slice(
            format!("{leaf_number} 0 obj\n<< /Type /Page /Parent {} 0 R >>\nendobj\n", leaf_number - 1)
                .as_bytes(),
        );

        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", total_objects + 1).as_bytes());
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF",
                total_objects + 1
            )
            .as_bytes(),
        );
        out
    }

    /// **Found by audit.** The depth cap alone lets a DAG double the walk at
    /// every level; a visited set collapses it back to one visit per node.
    #[test]
    fn a_page_tree_that_names_the_same_child_twice_is_walked_once_not_exponentially() {
        const DEPTH: u32 = 20;
        let bytes = dag_page_tree(DEPTH);
        let file = crate::pdf::File::parse(&bytes).expect("parse");
        let root = file
            .resolve(file.trailer().get(b"Root").expect("root"))
            .expect("resolve root");
        let pages = file
            .resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages"))
            .expect("resolve pages");

        let mut out = Vec::new();
        let started = std::time::Instant::now();
        collect_pages(&file, &pages, &mut out, &mut HashSet::new(), 0).expect("collect");
        let elapsed = started.elapsed();

        assert_eq!(out.len(), 1, "the same page reached twice at every level must be counted once");
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "collecting pages took {elapsed:?} — a DAG doubled the work at every level instead of \
             being deduplicated"
        );
    }
}

