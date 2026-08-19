//! Read-only [`Document`] backed by PDFium — the phase 1 implementation.

use std::fs::File;
use std::ffi::c_void;
use std::io::{Read, Seek, Write};
use std::os::raw::{c_int, c_uint, c_ulong};
use std::sync::OnceLock;

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPageRenderRotation,
    PdfPagePaperSize, PdfPoints, PdfRenderConfig, Pdfium, PdfiumLibraryBindingsAccessor,
    FPDFANNOT_COLORTYPE, FPDF_ANNOTATION, FPDF_ANNOTATION_SUBTYPE, FPDF_DOCUMENT, FPDF_FILEWRITE,
    FPDF_PAGE, FS_POINTF, FS_QUADPOINTSF, FS_RECTF,
};

use crate::document::metadata::DocumentMetadata;
use crate::document::{
    Annotation, Document, DocumentMut, Page, PageSize, Rect, RemovedPage, RenderRequest, Rotation,
    TextSegment,
};
use crate::error::{classify_pdfium_load_error, PdfError, Result};
use crate::render::bitmap::{self, Bitmap, PixelOrder};
use crate::render::RenderTarget;

/// The process-wide PDFium binding.
///
/// Leaked deliberately. PDFium's own `FPDF_InitLibrary`/`DestroyLibrary` pair is
/// process-global anyway, and a `&'static Pdfium` is what lets an open
/// `PdfDocument<'static>` live in the handle registry without a self-referential
/// struct. The leak is one allocation for the lifetime of the process.
static PDFIUM: OnceLock<std::result::Result<&'static Pdfium, String>> = OnceLock::new();

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
            let bindings = match std::env::var("PAGIFY_PDFIUM_LIB") {
                Ok(path) => Pdfium::bind_to_library(&path).map_err(|e| e.to_string())?,
                Err(_) => Pdfium::bind_to_system_library().map_err(|e| e.to_string())?,
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
    Memory { byte_len: usize },
}

pub struct PdfiumDocument {
    document: PdfDocument<'static>,
    source: DocumentSource,
    page_count: usize,
    /// Set by any mutation, so a caller can ask whether a save is owed.
    dirty: bool,
}

// `PdfDocument` already carries these under pdfium-render's `thread_safe` feature,
// which routes every PDFium call through a global mutex. `PdfiumDocument` adds no
// interior mutability of its own, so it inherits that guarantee unchanged.
unsafe impl Send for PdfiumDocument {}
unsafe impl Sync for PdfiumDocument {}

impl PdfiumDocument {
    pub fn open_path(path: &str, password: Option<&str>) -> Result<Self> {
        let file = File::open(path)?;
        Self::from_reader(file, password, DocumentSource::Path(path.to_string()))
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

    /// Uses `FPDF_GetPageSizeByIndexF`, which reads the page tree without
    /// loading the page itself.
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

    fn text(&self) -> Result<String> {
        Ok(self
            .page
            .text()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?
            .all())
    }

    fn text_segments(&self) -> Result<Vec<TextSegment>> {
        let page_height = self.page.height().value;
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
            // upwards; every consumer of this wants top-left with y increasing
            // down. Flipping once, here, keeps that conversion out of the UI.
            segments.push(TextSegment {
                left: bounds.left().value,
                top: page_height - bounds.top().value,
                right: bounds.right().value,
                bottom: page_height - bounds.bottom().value,
                text: content,
            });
        }
        Ok(segments)
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
    fn insert_blank_page(&mut self, at: usize, size: PageSize) -> Result<()> {
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

        Ok(match page.rotation().map_err(|e| PdfError::Pdfium(e.to_string()))? {
            PdfPageRenderRotation::Degrees90 => 1,
            PdfPageRenderRotation::Degrees180 => 2,
            PdfPageRenderRotation::Degrees270 => 3,
            _ => 0,
        })
    }

    fn add_annotation(&mut self, page_index: usize, annotation: &Annotation) -> Result<usize> {
        self.validate_page_index(page_index)?;
        let index = i32::try_from(page_index).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {page_index} is out of range"))
        })?;

        let page = RawPage::open(self.document.handle(), index)?;
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
        let annot_index = i32::try_from(index)
            .map_err(|_| PdfError::InvalidArgument(format!("annotation index {index} is out of range")))?;

        let page = RawPage::open(self.document.handle(), page_number)?;
        let removed =
            unsafe { pdfium()?.bindings().FPDFPage_RemoveAnnot(page.handle, annot_index) };
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
        let _ = index;
        // Removing is easy; *returning what was removed* is not, and without that
        // the removal cannot be undone. Reconstructing a mark from PDFium means
        // reading quad points, ink lists and colours back — the read half of this
        // module, which is not written yet. Refusing is the honest behaviour until
        // it is: an erase that worked with an undo that silently lost the mark
        // would be worse than one that does not start.
        Err(PdfError::Unsupported(
            "erasing a mark that is already saved in the file",
        ))
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

    fn import_pages(&mut self, _from: &dyn Document, _range: &[usize], _at: usize) -> Result<()> {
        // Needs the source document's raw handle, which is only reachable for a
        // `PdfiumDocument`; `&dyn Document` deliberately hides that. Landing this
        // means either downcasting or narrowing the parameter, and the choice
        // belongs with the merge feature that first needs it.
        Err(PdfError::Pdfium(
            "import_pages: needs the source document's PDFium handle; see DocumentMut".into(),
        ))
    }

    fn save_full_copy(&mut self, dest: &mut dyn Write) -> Result<()> {
        // A rewrite, and safe to build on the binding's own save: this is the
        // path that is *supposed* to relocate every object. Never the default —
        // it is what destroys a signature's byte range.
        let bytes = self
            .document
            .save_to_bytes()
            .map_err(|e| PdfError::Pdfium(e.to_string()))?;
        dest.write_all(&bytes)?;
        self.dirty = false;
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
        let bytes = self.save_with_flags(FPDF_INCREMENTAL)?;
        dest.write_all(&bytes)?;
        self.dirty = false;
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
    fn insert_page(&mut self, at: usize, page: RemovedPage) -> Result<()> {
        if at > self.page_count {
            return Err(PdfError::PageOutOfRange {
                index: at,
                count: self.page_count,
            });
        }
        let destination = i32::try_from(at).map_err(|_| {
            PdfError::InvalidArgument(format!("page index {at} is out of range"))
        })?;

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
            let at = current.iter().position(|&p| p == target).unwrap_or(position);
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

    /// Page height in points, which is what every coordinate flip below needs.
    fn height(&self) -> Result<f32> {
        Ok(unsafe { pdfium()?.bindings().FPDF_GetPageHeightF(self.handle) })
    }
}

impl Drop for RawPage {
    fn drop(&mut self) {
        if let Ok(pdfium) = pdfium() {
            unsafe { pdfium.bindings().FPDF_ClosePage(self.handle) };
        }
    }
}

/// Flip one y coordinate between the two conventions.
///
/// Ours is top-left with y down; PDF's is bottom-left with y up. The operation is
/// its own inverse, which is why one function serves both directions — and why
/// applying it twice by mistake is silent rather than obviously broken.
fn flip_y(page_height: f32, y: f32) -> f32 {
    page_height - y
}

/// One of our rects as PDFium wants it: bottom-left origin, `top` above `bottom`.
///
/// The ordering matters as much as the flip. Our `Rect` has `top < bottom`
/// because y grows downwards; after flipping, `top > bottom`. Handing PDFium an
/// inverted rect produces an annotation with no area, which draws as nothing at
/// all rather than as anything visibly wrong.
fn to_pdf_rect(page_height: f32, rect: &Rect) -> FS_RECTF {
    let top = flip_y(page_height, rect.top);
    let bottom = flip_y(page_height, rect.bottom);
    FS_RECTF {
        left: rect.left.min(rect.right),
        right: rect.left.max(rect.right),
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
        Annotation::Highlight { rects, .. } => {
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
    }
    bounds
}

impl PdfiumDocument {
    /// Write one mark onto an already-open page.
    ///
    /// Split out from `add_annotation` so the page stays open for exactly this
    /// call and the error paths all close it.
    fn write_annotation(&self, page: &RawPage, annotation: &Annotation) -> Result<()> {
        let bindings = pdfium()?.bindings();
        let height = page.height()?;

        let subtype = match annotation {
            Annotation::Highlight { .. } => ANNOT_HIGHLIGHT,
            Annotation::Ink { .. } => ANNOT_INK,
            Annotation::Note { .. } => ANNOT_TEXT,
        };

        // Safety: the page handle is live for the duration, and the annotation is
        // closed before returning on every path.
        let annot = unsafe { bindings.FPDFPage_CreateAnnot(page.handle, subtype) };
        if annot.is_null() {
            return Err(PdfError::Pdfium("could not create annotation".into()));
        }

        let outcome = self.fill_annotation(annot, annotation, height);

        unsafe { bindings.FPDFPage_CloseAnnot(annot) };
        outcome
    }

    fn fill_annotation(
        &self,
        annot: FPDF_ANNOTATION,
        annotation: &Annotation,
        height: f32,
    ) -> Result<()> {
        let bindings = pdfium()?.bindings();

        let bounds = bounding_box(annotation)
            .ok_or_else(|| PdfError::InvalidArgument("annotation has no geometry".into()))?;
        let rect = to_pdf_rect(height, &bounds);
        unsafe { bindings.FPDFAnnot_SetRect(annot, &rect) };

        let colour = match annotation {
            Annotation::Highlight { color, .. }
            | Annotation::Ink { color, .. }
            | Annotation::Note { color, .. } => *color,
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

        match annotation {
            Annotation::Highlight { rects, .. } => {
                for r in rects {
                    let pdf = to_pdf_rect(height, r);
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
                        .map(|p| FS_POINTF {
                            x: p.x,
                            y: flip_y(height, p.y),
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
