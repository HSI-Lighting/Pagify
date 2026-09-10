//! Read-only [`Document`] backed by PDFium — the phase 1 implementation.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::raw::{c_int, c_uint, c_ulong};
use std::sync::OnceLock;

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPagePaperSize,
    PdfPageObjectCommon, PdfPageObjectType, PdfPageObjectsCommon, PdfPageRenderRotation, PdfPoints,
    PdfRenderConfig, Pdfium, PdfiumLibraryBindingsAccessor,
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
    Annotation, Color, Document, DocumentMut, Glyph, IndexedAnnotation, Page, PageCharacters,
    PageClassification, PageSize, PageTextKind, RecognisedWord, TEXT_LAYER_ID,
    Point, Rect, RegionRequest, RemovedPage, RenderRequest, Rotation, Ruling, TextSegment,
    Redaction, RedactionReport, Uncleared,
};
use crate::crypto::vault::{self, Vault};
use crate::crypto::KdfParams;
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
                (Some(path), _) => Pdfium::bind_to_library(path).map_err(|e| e.to_string())?,
                (None, Ok(path)) => Pdfium::bind_to_library(&path).map_err(|e| e.to_string())?,
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
struct Wanted {
    user: Vec<u8>,
    owner: Option<Vec<u8>>,
    permissions: crate::pdf::encrypt::Permissions,
}

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
    vault: std::sync::Mutex<Option<Option<Vault>>>,
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
    /// So the bytes are kept at the moment they are produced — signing and
    /// timestamping both have them in hand — rather than reconstructed later
    /// from something that cannot reproduce them. A document opened from a
    /// path needs nothing here: its file is on disk and can be read back.
    /// `None` means "ask the source", and it is the ordinary case.
    written: Option<Vec<u8>>,
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
    opened_with: Option<Vec<u8>>,
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
}

// `PdfDocument` already carries these under pdfium-render's `thread_safe` feature,
// which routes every PDFium call through a global mutex. The one piece of
// interior mutability here — the cached vault — is behind a `Mutex` of its own,
// so the guarantee is inherited unchanged.
unsafe impl Send for PdfiumDocument {}
unsafe impl Sync for PdfiumDocument {}

impl PdfiumDocument {
    /// How many signatures a reader finds in this document.
    ///
    /// PDFium's own count, asked so a test can check that what was written is
    /// what somebody else's code sees — rather than only what ours does.
    pub fn signature_count(&self) -> i32 {
        let Ok(pdfium) = pdfium() else { return -1 };
        unsafe { pdfium.bindings().FPDF_GetSignatureCount(self.document.handle()) }
    }

    pub fn open_path(path: &str, password: Option<&str>) -> Result<Self> {
        // Read once to look for our own handler. A Secure Plus document has to
        // be unsealed here, because PDFium cannot read it — no reader can, which
        // is the point of it.
        if let Ok(bytes) = std::fs::read(path) {
            if let Some(unsealed) = Self::unseal_if_ours(&bytes, password)? {
                let mut doc = Self::open_bytes(unsealed, None)?;
                doc.source = DocumentSource::Path(path.to_string());
                doc.already_secured = true;
                doc.secure_plus = true;
                doc.opened_with = password.map(|p| p.as_bytes().to_vec());
                return Ok(doc);
            }
        }
        let file = File::open(path)?;
        Self::from_reader(file, password, DocumentSource::Path(path.to_string()))
    }

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
            doc.opened_with = password.map(|p| p.as_bytes().to_vec());
            return Ok(doc);
        }
        let byte_len = bytes.len();
        Self::from_reader(
            std::io::Cursor::new(bytes),
            password,
            DocumentSource::Memory { byte_len },
        )
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
            opened_with: password.filter(|p| !p.is_empty()).map(|p| p.as_bytes().to_vec()),
            security: None,
            redacted: false,
            written: None,
            typing_fonts: Vec::new(),
            substituted: None,
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

    /// Uses `FPDF_GetPageSizeByIndexF`, which reads the page tree without
    /// loading the page itself.
    fn text_runs(&self, page_index: usize) -> Result<Vec<crate::document::TextRun>> {
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
        // The text page is what turns a run's bytes into characters — the same
        // machinery extraction uses, so a run reads the way the rest of the
        // program reads the page.
        let text_page = page.text().map_err(|e| PdfError::Pdfium(e.to_string()))?;

        let mut runs = Vec::new();
        for (index, object) in page.objects().iter().enumerate() {
            let Some(text_object) = object.as_text_object() else { continue };
            let words = text_object.text();
            // Kept even when it reports no words.
            //
            // A run whose font has no `/ToUnicode` reads as empty here while
            // being perfectly visible on the page — which is most of "some
            // words are not recognised". Dropping those made them unclickable,
            // and a word you can see but cannot point at is worse than one
            // labelled unreadable.
            //
            // A run with no *area* is a different thing and is dropped: it
            // draws nothing, so there is nothing to have clicked on.
            let Ok(bounds) = object.bounds() else { continue };
            let wide = (bounds.right().value - bounds.left().value).abs() > 0.5;
            let tall = (bounds.top().value - bounds.bottom().value).abs() > 0.5;
            if !wide || !tall {
                continue;
            }
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
                size: text_object.unscaled_font_size().value,
                color: colour,
            });
        }
        drop(text_page);
        Ok(runs)
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
        if was != now {
            // Put the page back exactly as it was and say why.
            let restored = Self::open_bytes(snapshot, None)?;
            self.document = restored.document;
            self.page_count = restored.page_count;
            if let Ok(mut cached) = self.vault.lock() {
                *cached = None;
            }
            return Err(PdfError::Unsupported(
                "moving anything on this page rewrites its text, so nothing was moved",
            ));
        }

        self.dirty = true;
        Ok(())
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
        self.dirty = true;

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

        self.dirty = true;
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
        // Only when the words are all that is changing: a new colour, size or
        // position is PDFium's to apply, and it does that well.
        if *style == crate::document::TextStyle::default() {
            match self.set_run_in_stream(page_index, object, text) {
                Ok(previous) => {
                    return Ok((previous, crate::document::TextStyle::default()))
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
        let was = {
            let mut m = FS_MATRIX { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
            unsafe { bindings.FPDFPageObj_GetMatrix(handle, &mut m) };
            let (x, y) = space.to_top_left(m.e, m.f);
            let mut size = 0.0f32;
            unsafe { bindings.FPDFTextObj_GetFontSize(handle, &mut size) };
            crate::document::TextStyle {
                size: (size > 0.0).then_some(size),
                color: had_colour.then_some(Color {
                    r: r as u8,
                    g: g as u8,
                    b: b as u8,
                    a: a as u8,
                }),
                at: Some((x, y)),
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
            unsafe { bindings.FPDFTextObj_SetFontSize(handle, size) };
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
            return Err(PdfError::Unsupported(
                "changing how these words look rewrites the rest of the page, so \
                 nothing was changed — the words themselves can still be edited",
            ));
        }

        self.dirty = true;
        Ok((previous, was))
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
        self.dirty = true;
        Ok(())
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
        vault.hide_item(
            request.page_index,
            usize::MAX,
            [
                request.area.left,
                request.area.top,
                request.area.right,
                request.area.bottom,
            ],
        )?;
        self.write_vault(&vault)?;

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
        let hidden = self.hide_annotations_in(request.page_index, &request.area)?;

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
        let mut report = match self.cut_text_in_area(request) {
            Ok(report) => report,
            Err(_) => self.redact_inner(request, Intent::Apply, catalogue)?,
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
        self.write_vault(&vault)?;

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
        self.write_vault(&vault)?;

        // Rewritten now, not at save time: see `apply_locks`.
        self.apply_locks()?;
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
        self.write_vault(&vault)?;

        // The page came back whole, so everything else that is still locked on
        // it has to be hidden again — which `apply_locks` does for the whole
        // document from the vault, through the same rewrite.
        self.apply_locks()?;
        Ok(())
    }

    fn locked_items_on(&self, page_index: usize) -> Result<Vec<crate::document::LockedItem>> {
        let Some(vault) = self.read_vault()? else { return Ok(Vec::new()) };
        Ok(vault
            .items_on(page_index)
            .into_iter()
            .map(|i| crate::document::LockedItem {
                id: i.id.clone(),
                rect: crate::document::Rect {
                    left: i.rect[0],
                    top: i.rect[1],
                    right: i.rect[2],
                    bottom: i.rect[3],
                },
                // `usize::MAX` is how a locked region is recorded — it names no
                // object, because it is not one.
                is_area: i.object == usize::MAX,
            })
            .collect())
    }

    fn open_lock(&mut self, passcode: &[u8]) -> Result<Vec<(usize, Vec<u8>)>> {
        let Some(vault) = self.read_vault()? else {
            return Err(PdfError::InvalidArgument("this document is not locked".into()));
        };
        let dek = vault.unlock(passcode)?;

        // Every tag verified before a single page is handed back. One damaged
        // seal fails the whole call rather than restoring some pages and leaving
        // nobody able to say which are which.
        vault
            .locked_pages()
            .into_iter()
            .map(|index| vault.open_page(&dek, index).map(|bytes| (index, bytes)))
            .collect()
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
            user: user.to_vec(),
            owner: owner.map(<[u8]>::to_vec),
            permissions,
        });
        // PDF's own handler, so any reader can ask for it.
        self.secure_plus = false;
        self.dirty = true;
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
            user: user.to_vec(),
            owner: None,
            permissions: crate::pdf::encrypt::Permissions::all(),
        });
        self.secure_plus = true;
        self.dirty = true;
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
        self.dirty = true;
        Ok(())
    }

    fn is_secured(&self) -> bool {
        self.security.is_some()
    }

    fn had_password_on_open(&self) -> bool {
        self.already_secured
    }

    fn password_matches(&self, typed: &[u8]) -> bool {
        self.opened_with.as_deref() == Some(typed)
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
        // **Saving again would break it.** The signature is over the bytes as
        // written; anything that rewrites them invalidates it. Marked clean so
        // the obvious next action is not the one that undoes the work.
        self.dirty = false;
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
        let bytes = match (&self.written, &self.source) {
            (Some(exact), _) => exact.clone(),
            (None, DocumentSource::Path(path)) => std::fs::read(path)?,
            // No file to read and no bytes kept — say that rather than check
            // something else and present the answer as being about this.
            (None, _) => {
                return Err(PdfError::Unsupported(
                    "checking signatures on a document that has never been written",
                ))
            }
        };
        let file = crate::pdf::File::parse(&bytes)?;
        crate::pdf::validate::check(&file, &bytes)
    }

    fn timestamp_document(&mut self, authority: &str) -> Result<()> {
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let stamped = crate::pdf::sign::timestamp(&file, authority)?;

        let exact = stamped.clone();
        let reopened = Self::open_bytes(stamped, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        self.written = Some(exact);
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        // As with a signature: it covers the bytes as written, so the obvious
        // next action must not be the one that undoes it.
        self.dirty = false;
        Ok(())
    }

    fn set_typing_fonts(&mut self, fonts: Vec<Vec<u8>>) {
        self.typing_fonts = fonts;
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
        if subtype != ANNOT_INK {
            unsafe { bindings.FPDFPage_CloseAnnot(annot) };
            return Err(PdfError::InvalidArgument(
                "only ink can be marked as a signature".into(),
            ));
        }

        let mut value: Vec<u16> = name.encode_utf16().collect();
        value.push(0);
        let set = unsafe { bindings.FPDFAnnot_SetStringValue(annot, SIGNATURE_KEY, value.as_ptr()) };
        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        if set == 0 {
            return Err(PdfError::Pdfium("the signature could not be marked".into()));
        }

        self.dirty = true;
        Ok(())
    }

    fn apply_signatures(&mut self, page_index: usize) -> Result<usize> {
        let marks = self.signature_marks(page_index)?;
        if marks.is_empty() {
            return Ok(0);
        }
        let height = self.page_size(page_index)?.height_pt;

        // One append for the whole page rather than one per signature: each
        // append writes the document out and parses it again, which on a large
        // file is the expensive part and has nothing to do with how many
        // signatures are on the page.
        let mut painted = Vec::new();
        for mark in &marks {
            painted.extend_from_slice(&ink_operators(&mark.strokes, mark.color, mark.width, height));
        }
        self.append_to_page(page_index, &painted)?;

        // From the back: removing by index renumbers everything after it.
        let mut indices: Vec<usize> = marks.iter().map(|m| m.index).collect();
        indices.sort_unstable();
        for index in indices.into_iter().rev() {
            self.remove_annotation(page_index, index)?;
        }
        Ok(marks.len())
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

        self.dirty = true;
        Ok(())
    }

    fn clear_sensitivity(&mut self) -> Result<()> {
        use crate::document::sensitivity::STAMP_ID;

        for page_index in 0..self.page_count {
            // Absent on most pages most of the time; not an error.
            let _ = self.remove_text(page_index, STAMP_ID);
        }
        self.dirty = true;
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
        crate::pdf::hidden::survey(&file, &bytes)
    }

    fn remove_hidden_data(&mut self) -> Result<crate::pdf::hidden::Hidden> {
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        let file = crate::pdf::File::parse(&bytes)?;
        let (cleaned, found) = crate::pdf::hidden::strip(&file, &bytes)?;

        // Reopened from the sanitised bytes, so what the person is looking at
        // is the document they just cleaned.
        let reopened = Self::open_bytes(cleaned, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        self.dirty = true;
        // The file now holds one revision, and appending to it would start the
        // problem over: an earlier version of every page, in front of the new
        // one.
        self.redacted = true;
        Ok(found)
    }

    fn must_save_full_copy(&self) -> bool {
        // A password cannot be appended: an incremental save leaves the whole
        // original revision in the file, in plain sight, with the encrypted one
        // after it.
        self.redacted || self.security.is_some()
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
        self.dirty = true;
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
        self.dirty = true;
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
            self.dirty = true;
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

        self.dirty = true;
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

        self.dirty = true;
        Ok(())
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

    fn remove_text(&mut self, page_index: usize, id: i32) -> Result<()> {
        self.validate_page_index(page_index)?;
        let page_number = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let bindings = pdfium()?.bindings();
        let mut taken = 0;

        // Safety: the page is live for the loop; every object removed is destroyed
        // exactly once and never touched again.
        unsafe {
            // Backwards, because removing an object renumbers everything after it.
            for index in (0..bindings.FPDFPage_CountObjects(page.handle)).rev() {
                let object = bindings.FPDFPage_GetObject(page.handle, index);
                if object.is_null() || text_mark_id(bindings, object) != Some(id) {
                    continue;
                }
                if bindings.FPDFPage_RemoveObject(page.handle, object) != 0 {
                    bindings.FPDFPageObj_Destroy(object);
                    taken += 1;
                }
            }

            if taken == 0 {
                return Err(PdfError::InvalidArgument(format!(
                    "page {page_index} has no text mark {id}"
                )));
            }

            if bindings.FPDFPage_GenerateContent(page.handle) == 0 {
                return Err(PdfError::Pdfium(
                    "text was removed but the page content was not regenerated".into(),
                ));
            }
        }

        Ok(())
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

        let bytes = close_trailing_xref_object(self.save_with_flags(FPDF_INCREMENTAL)?);
        dest.write_all(&bytes)?;
        self.dirty = false;
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
        self.dirty = true;
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
        self.dirty = true;
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

        self.dirty = true;
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
const ANNOT_HIGHLIGHT: FPDF_ANNOTATION_SUBTYPE = 9;
const ANNOT_UNDERLINE: FPDF_ANNOTATION_SUBTYPE = 10;
const ANNOT_SQUIGGLY: FPDF_ANNOTATION_SUBTYPE = 11;
const ANNOT_STRIKEOUT: FPDF_ANNOTATION_SUBTYPE = 12;
const ANNOT_INK: FPDF_ANNOTATION_SUBTYPE = 15;

/// `FPDFANNOT_COLORTYPE_Color` — the stroke/foreground colour.
const COLORTYPE_COLOR: FPDFANNOT_COLORTYPE = 0;

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
            .filter(|r| {
                r.rect.left < request.area.right
                    && r.rect.right > request.area.left
                    && r.rect.top < request.area.bottom
                    && r.rect.bottom > request.area.top
            })
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
        let mut edits: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
        let mut cut_whole: Vec<usize> = Vec::new();
        let mut spilled: Vec<String> = Vec::new();
        // Counted as it goes, because a sliced run loses only the glyphs the
        // selection covered while one cut whole loses all of them.
        let mut characters = 0usize;

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

            let widths: Vec<Option<usize>> = parts
                .iter()
                .map(|p| {
                    p.font
                        .as_ref()
                        .zip(fonts.as_ref())
                        .and_then(|(name, dict)| code_width(&file, dict, name))
                })
                .collect();
            let counts: Vec<usize> = parts
                .iter()
                .zip(&widths)
                .map(|(p, width)| {
                    let w = width.unwrap_or(1).max(1);
                    content::pieces(&operations[p.origin.operation])
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
            let wanted = run.text.chars().count();
            let spellings: Vec<Option<String>> = parts
                .iter()
                .zip(&widths)
                .flat_map(|(part, width)| {
                    let map = part
                        .font
                        .as_ref()
                        .zip(fonts.as_ref())
                        .and_then(|(name, dict)| Self::font_to_unicode(&file, &bytes, dict, name));
                    codes_of(&operations[part.origin.operation], width.unwrap_or(1))
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
            let owner = align_codes(&spellings, &run.text).or_else(|| {
                (spellings.len() == wanted).then(|| (0..wanted).collect())
            });

            // The glyphs the run drew — one per character it reported. Found
            // by where the run starts, nearest rather than
            // within-a-threshold: a run's rectangle bounds its ink while a
            // glyph's box carries its side bearing, so the corners never quite
            // coincide, and a tight threshold found no start at all on fifteen
            // of sixteen real pages.
            let run_glyph = glyph_starting_at(run.rect.left, run.rect.bottom);
            let mine: Vec<(usize, crate::document::Rect)> = if indexable && wanted > 0 {
                run_glyph
                    .filter(|from| from + wanted <= per_char)
                    .map(|from| (from..from + wanted).map(|i| (i, box_at(i))).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // Which characters the selection covers.
            let covered_chars: Vec<usize> = mine
                .iter()
                .enumerate()
                .filter(|(_, (_, r))| {
                    r.left < request.area.right
                        && r.right > request.area.left
                        && r.top < request.area.bottom
                        && r.bottom > request.area.top
                })
                .map(|(n, _)| n)
                .collect();

            let sliceable = widths.iter().all(Option::is_some)
                && owner.is_some()
                && !mine.is_empty()
                && !covered_chars.is_empty()
                && covered_chars.len() < mine.len();

            if sliceable {
                let owner = owner.expect("checked");

                // Where each code's characters begin, and therefore which codes
                // the covered characters belong to.
                let mut first_char: std::collections::BTreeMap<usize, usize> = Default::default();
                for (character, code) in owner.iter().enumerate() {
                    if *code != usize::MAX {
                        first_char.entry(*code).or_insert(character);
                    }
                }
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

                // How far each dropped code advanced the pen: from where its
                // characters start to where the next code's do. Measured across
                // the glyphs rather than from font metrics, so side bearings and
                // character spacing are already in it.
                let starts: Vec<(usize, usize)> =
                    first_char.iter().map(|(c, ch)| (*c, *ch)).collect();
                let advance_of = |code: usize| -> f32 {
                    let Some(position) = starts.iter().position(|(c, _)| *c == code) else {
                        // Owns no character, so it moved the pen by nothing that
                        // can be seen.
                        return 0.0;
                    };
                    let from = starts[position].1;
                    match starts.get(position + 1) {
                        Some((_, next)) => mine[*next].1.left - mine[from].1.left,
                        None => mine[mine.len() - 1].1.right - mine[from].1.left,
                    }
                };

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
                    let operation = &operations[part.origin.operation];
                    let font = part.font.clone().unwrap_or_default();
                    edits.push((
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
                characters += covered_chars.len();
            } else {
                characters += run.text.chars().count();
                for part in &parts {
                    cut_whole.push(part.origin.operation);
                }
                // Only when something went that was *not* asked for. A run
                // wholly inside the selection loses nothing extra by being cut
                // whole, and reporting it would tell the caller a line vanished
                // when it did not.
                let beyond = run.rect.left < request.area.left - 0.5
                    || run.rect.right > request.area.right + 0.5
                    || run.rect.top < request.area.top - 0.5
                    || run.rect.bottom > request.area.bottom + 0.5;
                if beyond {
                    spilled.push(run.text.clone());
                }
            }
        }

        // Anything else starting inside the area — the hyphens and fragments
        // PDFium folds into a neighbouring run rather than reporting
        // separately, which would otherwise be left behind. Only where nothing
        // is being sliced there, or the two edits would fight over the bytes.
        for p in &placed {
            let (x, y) = (p.origin.x, height - p.origin.y);
            if x >= request.area.left
                && x <= request.area.right
                && y >= request.area.top
                && y <= request.area.bottom
                && !edits.iter().any(|(span, _)| *span == operations[p.origin.operation].span)
            {
                cut_whole.push(p.origin.operation);
            }
        }
        cut_whole.sort_unstable();
        cut_whole.dedup();
        for index in &cut_whole {
            edits.push((operations[*index].span.clone(), Vec::new()));
        }

        let mut edited = content::splice(&stream, &edits);
        if let Some(fill) = request.fill {
            edited.extend_from_slice(&mark(request.area, height, fill));
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
        self.dirty = true;
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
        for (key, _) in fonts.0.iter() {
            if key == own {
                continue;
            }
            let Some(width) = code_width(file, fonts, key) else { continue };
            let map = match Self::font_to_unicode(file, bytes, fonts, key) {
                Some(map) => map,
                None if width == 1 => (0x20u32..0x7F)
                    .filter_map(|code| char::from_u32(code).map(|c| (code, c.to_string())))
                    .collect(),
                None => continue,
            };
            let mut reverse: std::collections::BTreeMap<String, u32> = Default::default();
            for (code, spelling) in &map {
                reverse.entry(spelling.clone()).or_insert(*code);
            }
            if let Some(encoded) = encode_with(&reverse, wanted, width) {
                return Some((
                    Swapped {
                        resource: key.clone(),
                        face: format!("/{}", String::from_utf8_lossy(key)),
                        added: Vec::new(),
                        page: None,
                    },
                    encoded,
                ));
            }
        }
        None
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
    ) -> Result<(Swapped, Vec<u8>)> {
        use crate::pdf::{embed, Object};

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

        let first = file.numbers().max().unwrap_or(0) + 1;
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
        self.dirty = true;
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
        use crate::pdf::content;

        let pictures = self.images_on(page_index)?;
        let Some(which) = pictures.iter().position(|i| i.object == object) else {
            return Err(PdfError::InvalidArgument("that is not a picture".into()));
        };

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let bytes = self.readable_bytes()?;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;

        // Which names in this page's resources are images, so a `Do` that draws
        // a form is not mistaken for one that draws a picture.
        let images: std::collections::BTreeSet<Vec<u8>> = self
            .inherited(&file, &page, b"Resources")?
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
            .unwrap_or_default();

        let drawn: Vec<&content::Operation> = operations
            .iter()
            .filter(|op| {
                op.operator == b"Do"
                    && matches!(
                        op.operands.first(),
                        Some(crate::pdf::Object::Name(name)) if images.contains(name)
                    )
            })
            .collect();

        // The evidence: as many pictures drawn as PDFium reports.
        if drawn.len() != pictures.len() {
            return Err(PdfError::Unsupported(
                "this page draws its pictures in a way this cannot follow",
            ));
        }
        let target = drawn[which];

        // **The distance has to be expressed in the space the operator sits
        // in.** A picture is placed by a `cm` that scales the unit square up to
        // its size on the page, so a translation written *inside* that is
        // multiplied by the picture's own width and height — measured, twenty
        // points came out as three thousand nine hundred. So the walk below
        // rebuilds the transform in force at that operator, and the move is
        // carried back through it.
        let ctm = ctm_at(&operations, target);
        let (det, wanted_x, wanted_y) = (
            ctm[0] * ctm[3] - ctm[1] * ctm[2],
            by.x,
            // Page space counts downwards, a content stream upwards.
            -by.y,
        );
        if det.abs() < 1e-9 {
            return Err(PdfError::Unsupported(
                "this picture is placed by a transform this cannot invert",
            ));
        }
        let local = (
            (ctm[3] * wanted_x - ctm[2] * wanted_y) / det,
            (-ctm[1] * wanted_x + ctm[0] * wanted_y) / det,
        );

        let wrapped = format!(
            "q 1 0 0 1 {} {} cm\n{}\nQ",
            local.0,
            local.1,
            String::from_utf8_lossy(&stream[target.span.clone()])
        );
        let edited = content::splice(&stream, &[(target.span.clone(), wrapped.into_bytes())]);

        let mut replacements = Vec::new();
        for (index, (number, dict)) in streams.iter().enumerate() {
            let data = if index == 0 { edited.clone() } else { Vec::new() };
            let packed = content::encode(&data)?;
            let mut dict = dict.clone();
            dict.set(b"Filter", crate::pdf::Object::Name(b"FlateDecode".to_vec()));
            dict.remove(b"DecodeParms");
            replacements.push((*number, crate::pdf::write_stream(&dict, &packed)));
        }

        let rewritten = file.rewrite(&replacements)?;
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        self.rearm_security(was_secured, plus, permissions);
        self.dirty = true;
        Ok(())
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
        self.set_run_in_stream(page_index, object, text).map(|_| ())
    }

    /// Change one run's words by editing the content stream.
    ///
    /// Returns what the run said before. Refuses — leaving the page untouched —
    /// wherever it cannot be certain: a filter it cannot read, a run whose
    /// operator cannot be located, a font with no `/ToUnicode` to encode the
    /// new text through, or a character that font cannot spell. The caller then
    /// falls back to PDFium, which always works and costs the page's layout.
    fn set_run_in_stream(
        &mut self,
        page_index: usize,
        object: usize,
        text: &str,
    ) -> Result<String> {
        use crate::pdf::content;

        let run_list = self.text_runs(page_index)?;
        let run = run_list
            .iter()
            .find(|r| r.object == object)
            .cloned()
            .ok_or_else(|| PdfError::InvalidArgument("that is not a text run".into()))?;
        let previous = run.text.clone();
        let height = self.page_size(page_index)?.height_pt;
        self.substituted = None;

        // Readable by *this* crate, which an encrypted document is not until
        // its security comes off — see `readable_bytes`.
        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let bytes = self.readable_bytes()?;
        let file = crate::pdf::File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let fonts = self.page_fonts(&file, &page);

        let operations = content::parse(&stream)?;
        let placed = content::placed(&operations);

        let (want_x, want_y) = (run.rect.left, height - run.rect.bottom);
        let found = match nearest_placed(&placed, want_x, want_y) {
            Some((at, _)) => &placed[at],
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
                let index = self
                    .text_runs(page_index)?
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
        let unicode = match Self::font_to_unicode(&file, &bytes, &fonts, &name) {
            Some(map) => map,
            None if width == 1 => (0x20u32..0x7F)
                .filter_map(|code| {
                    char::from_u32(code).map(|c| (code, c.to_string()))
                })
                .collect(),
            None => {
                return Err(PdfError::Unsupported(
                    "that font carries no character map this can read",
                ))
            }
        };

        // The map read backwards: what code spells each character. Where two
        // codes spell the same thing the lower one wins, which keeps the choice
        // stable rather than dependent on iteration order.
        let mut reverse: std::collections::BTreeMap<String, u32> = Default::default();
        for (code, spelling) in &unicode {
            reverse.entry(spelling.clone()).or_insert(*code);
        }

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
        let (encoded, swap) = match encode_with(&reverse, &wanted, width) {
            Some(bytes) => (bytes, None),
            None => match self.borrow_font_on_page(&file, &bytes, &fonts, &name, &wanted) {
                Some((swap, encoded)) => (encoded, Some(swap)),
                None => {
                    let (swap, encoded) =
                        self.embed_typing_font(&file, page_index, &page, &wanted, &reverse)?;
                    (encoded, Some(swap))
                }
            },
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
        let mut replacement = content::replacing_codes(
            operation,
            first..last + 1,
            &encoded,
            &drawing,
            found.size,
            width,
        );
        if swap.is_some() {
            // **Put the page's own font back.** `Tf` is graphics state: it
            // stays selected until something changes it, so anything drawn
            // after this in the same text object would come out in a face
            // nobody asked for.
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

        let rewritten = file.rewrite_adding(&replacements, &extra, &crate::pdf::Dict(Vec::new()))?;
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        // The password the read had to take off.
        self.rearm_security(was_secured, plus, permissions);
        // Recorded rather than returned, so the caller can say what happened
        // without every signature in the chain growing a field for it.
        self.substituted = swap.map(|s| s.face);
        self.dirty = true;
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
        if !was_secured {
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
            wanted.owner.as_deref(),
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
        let bytes = self.readable_bytes()?;
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
                let number = file.numbers().max().unwrap_or(0) + 1;
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

        let rewritten =
            file.rewrite_adding(&replacements, &extra, &crate::pdf::Dict(Vec::new()))?;
        let reopened = Self::open_bytes(rewritten, None)?;
        self.document = reopened.document;
        self.page_count = reopened.page_count;
        if let Ok(mut cached) = self.vault.lock() {
            *cached = None;
        }
        self.rearm_security(was_secured, plus, permissions);
        self.dirty = true;
        Ok(())
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
        self.dirty = true;
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

        // The record keeps PDFium's object index, which counts every object on
        // the page; the resource dictionary is keyed by name and unordered. So
        // the two are matched on what the image *is* — the pixel size PDFium
        // reported when it was locked — rather than on position.
        let locked = self
            .images_on(item.page_index)?
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
        collect_pages(file, &pages, &mut flat, 0)?;
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
                let Ok(resolved) = file.object(number) else { continue };
                let is_page = resolved
                    .as_dict()
                    .and_then(|d| d.get(b"Type"))
                    .and_then(crate::pdf::Object::as_name)
                    == Some(&b"Page"[..]);
                if is_page {
                    out.push(number);
                } else {
                    walk(file, &resolved, out, depth + 1);
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
        walk(file, &pages, &mut numbers, 0);
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
        if let Some(vault) = cached.as_ref() {
            return Ok(vault.clone());
        }
        let read = self.read_vault_uncached()?;
        *cached = Some(read.clone());
        Ok(read)
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
    fn write_vault(&mut self, vault: &Vault) -> Result<()> {
        // What was cached is now what is being replaced.
        if let Ok(mut cached) = self.vault.lock() {
            *cached = Some(Some(vault.clone()));
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

        self.dirty = true;
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

/// Every page in a page tree, in order.
///
/// Walked rather than indexed: `/Kids` may nest, and a tree of a hundred and
/// forty-nine pages is rarely flat.
fn collect_pages(
    file: &crate::pdf::File<'_>,
    node: &crate::pdf::Object,
    out: &mut Vec<crate::pdf::Object>,
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
            let kid = file.resolve(&kid)?;
            collect_pages(file, &kid, out, depth + 1)?;
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
        let mut index_of: HashMap<usize, usize> = HashMap::new();

        let count = unsafe { bindings.FPDFPage_CountObjects(raw.handle) };
        let mut queue: Vec<(FPDF_PAGEOBJECT, bool)> = Vec::new();
        for i in 0..count {
            let handle = unsafe { bindings.FPDFPage_GetObject(raw.handle, i) };
            if !handle.is_null() {
                queue.push((handle, false));
            }
        }

        // Depth-limited: a form may legally reference another, and a damaged
        // file may do so in a circle.
        let mut depth = 0;
        while !queue.is_empty() && depth < 8 {
            let mut next = Vec::new();
            for (handle, nested) in queue.drain(..) {
                index_of.insert(handle as usize, objects.len());
                objects.push(handle);
                inside_form.push(nested);

                if unsafe { bindings.FPDFPageObj_GetType(handle) } as u32 == FPDF_PAGEOBJ_FORM {
                    let children = unsafe { bindings.FPDFFormObj_CountObjects(handle) };
                    for i in 0..children {
                        let child = unsafe { bindings.FPDFFormObj_GetObject(handle, i as c_ulong) };
                        if !child.is_null() {
                            next.push((child, true));
                        }
                    }
                }
            }
            queue = next;
            depth += 1;
        }

        let mut marks: HashMap<usize, Vec<Marked>> = HashMap::new();
        let mut texts: HashMap<usize, String> = HashMap::new();
        let mut unreadable: HashSet<usize> = HashSet::new();
        let mut report = RedactionReport::default();
        let mut nested_text = false;

        let text_page = unsafe { bindings.FPDFText_LoadPage(raw.handle) };
        if !text_page.is_null() {
            let chars = unsafe { bindings.FPDFText_CountChars(text_page) };
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
                let Some(&object) = index_of.get(&(owner as usize)) else {
                    // A character whose object is not in the page's own list
                    // lives inside a form XObject, and cannot be addressed for
                    // removal from here.
                    nested_text = true;
                    continue;
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
                    continue;
                }

                let (left, top) = space.to_top_left(l as f32, t as f32);
                let (right, bottom) = space.to_top_left(r as f32, b as f32);
                let glyph = Rect { left, top, right, bottom };
                marks
                    .entry(object)
                    .or_default()
                    .push(Marked { covered: overlaps(&glyph, &area), left: l as f32 });
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

        for (position, &handle) in objects.iter().enumerate() {
            let kind = unsafe { bindings.FPDFPageObj_GetType(handle) } as u32;

            if kind == FPDF_PAGEOBJ_TEXT {
                let Some(marked) = marks.get(&position) else { continue };
                let text = texts.get(&position).cloned().unwrap_or_default();

                // Inside a shared form: seen, reported, and left alone.
                if inside_form[position] {
                    if marked.iter().any(|m| m.covered) {
                        report.uncleared.push(Uncleared::Form { object: position });
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
            return Err(PdfError::Unsupported(
                "nothing in this area is text — the words are part of an image, so a \
                 redaction here would draw a mark and remove nothing",
            ));
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

        self.dirty = true;
        // Set even when the rectangle matched nothing. Whether an incremental
        // save is safe is not a judgement to make from one rectangle's yield.
        self.redacted = true;
        Ok(report)
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

        let colour = match annotation {
            Annotation::Highlight { color, .. }
            | Annotation::Underline { color, .. }
            | Annotation::StrikeOut { color, .. }
            | Annotation::Squiggly { color, .. }
            | Annotation::Ink { color, .. }
            | Annotation::Note { color, .. }
            | Annotation::Text { color, .. } => *color,
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
            Annotation::Note { contents, .. } => {
                unsafe { bindings.FPDFAnnot_SetStringValue_str(annot, "Contents", contents) };
            }
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

            // Widgets, links, stamps, everything else: left exactly as they are.
            _ => return Ok(None),
        };

        Ok(Some(annotation))
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
    let packed = read_annotation_string(annot, COLOUR_KEY)?;
    if packed.len() != 8 {
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
        if !contains(&bytes[dict..], b"/Type") {
            let mut typed = Vec::with_capacity(bytes.len() + 12);
            typed.extend_from_slice(&bytes[..dict]);
            typed.extend_from_slice(b"/Type/XRef");
            typed.extend_from_slice(&bytes[dict..]);
            bytes = typed;
        }
    }

    bytes
}

/// Byte just after the `<<` opening the trailing cross-reference stream's
/// dictionary, if there is one.
///
/// Inserting there is offset-safe for the same reason closing the object is: the
/// only offset naming this object is `startxref`, which points at its *header* —
/// before the insertion — and every offset the table itself holds points at
/// objects earlier in the file. Nothing that is pointed at moves.
fn trailing_dictionary_start(bytes: &[u8]) -> Option<usize> {
    let startxref = find_last(bytes, b"startxref")?;
    let stream = find_last(&bytes[..startxref], b"stream")?;
    let open = find_last(&bytes[..stream], b"<<")?;
    Some(open + 2)
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
    let characters = (length as usize / 2).saturating_sub(1);
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
        self.dirty = true;
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
}
