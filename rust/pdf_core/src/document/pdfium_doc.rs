//! Read-only [`Document`] backed by PDFium — the phase 1 implementation.

use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::raw::{c_int, c_uint, c_ulong};
use std::sync::OnceLock;

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPagePaperSize,
    PdfPageRenderRotation, PdfPoints, PdfRenderConfig, Pdfium, PdfiumLibraryBindingsAccessor,
    FPDFANNOT_COLORTYPE, FPDF_ANNOTATION, FPDF_ANNOTATION_SUBTYPE, FPDF_DOCUMENT, FPDF_FILEWRITE,
    FPDF_FONT, FPDF_PAGE, FPDF_PAGEOBJECT, FPDF_PAGEOBJECTMARK, FS_POINTF, FS_QUADPOINTSF,
    FS_RECTF,
};
use pdfium_render::prelude::PdfiumLibraryBindings;

use crate::document::metadata::DocumentMetadata;
use crate::document::{
    Annotation, Color, Document, DocumentMut, Glyph, IndexedAnnotation, Page, PageCharacters,
    PageSize,
    Point, Rect, RegionRequest, RemovedPage, RenderRequest, Rotation, Ruling, TextSegment,
};
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
    Memory {
        byte_len: usize,
    },
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

    /// Uses `FPDF_GetPageSizeByIndexF`, which reads the page tree without
    /// loading the page itself.
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

            segments.push(TextSegment {
                left,
                top,
                right,
                bottom,
                text: content,
            });
        }
        Ok(segments)
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
        let bytes = close_trailing_xref_object(self.save_with_flags(FPDF_INCREMENTAL)?);
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
            Annotation::Highlight { rects, .. } => {
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
            ANNOT_HIGHLIGHT => {
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
                Annotation::Highlight {
                    rects,
                    color: colour,
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
