//! The C ABI surface. The iOS counterpart of [`crate::jni_bridge`].
//!
//! Built for iOS, and **also for macOS** so the host suite can exercise the exact
//! bytes iOS will call. A bridge that is only compiled on the device is a bridge
//! whose ownership contracts are only tested on the device.
//!
//! Three rules hold for every exported function here, and they are the same three
//! the JNI side keeps:
//!
//! 1. The body runs inside [`guard`], so a Rust panic becomes an error code
//!    instead of unwinding into Swift (which is undefined behaviour). The release
//!    profile keeps `panic = "unwind"` for exactly this.
//! 2. Errors are reported out of band: the call returns a sentinel and the
//!    message is fetched with [`pagify_last_error_message`]. C has no exceptions,
//!    so the sentinel *is* the signal — every one of them is a value the happy
//!    path cannot produce.
//! 3. Ownership is stated per pointer and never shared. Strings and buffers this
//!    module returns are freed by [`pagify_string_free`] /
//!    [`pagify_buffer_free`], never by `free(3)`: they came from Rust's
//!    allocator, and on iOS that is not the same allocator Swift would hand them
//!    to.
//!
//! File descriptors keep the JNI contract unchanged: `..._fd` entry points adopt
//! the descriptor and the callee always closes it, on every path including the
//! failing ones.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::AssertUnwindSafe;

use crate::command::Command;
use crate::document::blank::{blank_document, Ruling};
use crate::document::pdfium_doc::PdfiumDocument;
use crate::document::{
    Color, Document, PageSize, Point, Rect, RegionRequest, RenderRequest, Rotation,
};
use crate::engine;
use crate::error::{PdfError, Result};
use crate::registry;
use crate::render::{self, ImageFormat, Markup, PixelOrder, RenderTarget, Tile, ViewportRequest};

/// The handle a failed open returns. Matches `INVALID_HANDLE` on the JNI side.
pub const PAGIFY_INVALID_HANDLE: i64 = -1;
/// Returned by anything reporting only success or failure.
pub const PAGIFY_OK: i32 = 0;
pub const PAGIFY_ERROR: i32 = -1;

// ------------------------------------------------------------------ errors --

thread_local! {
    /// The last error on *this* thread. Thread-local rather than global because
    /// the reader renders on a background queue while the UI queue is opening
    /// documents, and a shared slot would let one thread read the other's
    /// failure.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(message: &str) {
    // A NUL inside the message would truncate it; replace rather than drop the
    // message, since this is the only diagnostic the caller gets.
    let sanitised = message.replace('\0', "?");
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = CString::new(sanitised).ok();
    });
}

/// The message for the most recent failure on this thread, or null if the last
/// call succeeded.
///
/// **Caller owns the returned string** and must release it with
/// [`pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_last_error_message() -> *mut c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(message) => message.clone().into_raw(),
        None => std::ptr::null_mut(),
    })
}

/// Run a fallible body, converting both errors and panics into `fallback` plus a
/// message on [`pagify_last_error_message`].
///
/// The slot is cleared on entry, so a stale message from an earlier call can
/// never be read as this call's failure.
fn guard<T>(fallback: T, body: impl FnOnce() -> Result<T>) -> T {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);

    match std::panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            set_last_error(&error.to_string());
            fallback
        }
        Err(payload) => {
            let message = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic payload".to_string()
            };
            log::error!("panic crossing the C boundary: {message}");
            set_last_error(&PdfError::Panic(message).to_string());
            fallback
        }
    }
}

// ------------------------------------------------------------------ memory --

/// Release a string returned by this module.
///
/// # Safety
/// `value` must be a pointer this module returned and not yet freed, or null.
#[no_mangle]
pub unsafe extern "C" fn pagify_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

/// An owned byte buffer handed to the caller. `cap` is carried because Rust's
/// allocator needs the original capacity to release it, and a `Vec` whose
/// capacity exceeds its length is the normal case for an encoded image.
#[repr(C)]
pub struct PagifyBuffer {
    pub data: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl PagifyBuffer {
    fn empty() -> Self {
        PagifyBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    fn from_vec(mut bytes: Vec<u8>) -> Self {
        let buffer = PagifyBuffer {
            data: bytes.as_mut_ptr(),
            len: bytes.len(),
            cap: bytes.capacity(),
        };
        std::mem::forget(bytes);
        buffer
    }
}

/// Release a buffer returned by this module.
///
/// # Safety
/// `buffer` must be one this module returned, freed at most once.
#[no_mangle]
pub unsafe extern "C" fn pagify_buffer_free(buffer: PagifyBuffer) {
    if !buffer.data.is_null() {
        drop(unsafe { Vec::from_raw_parts(buffer.data, buffer.len, buffer.cap) });
    }
}

// ----------------------------------------------------------------- helpers --

/// # Safety
/// `value` must be null or a NUL-terminated string valid for this call.
unsafe fn optional_str<'a>(value: *const c_char) -> Result<Option<&'a str>> {
    if value.is_null() {
        return Ok(None);
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|e| PdfError::InvalidArgument(format!("string is not valid UTF-8: {e}")))?;
    Ok(Some(text))
}

/// # Safety
/// As [`optional_str`], and `value` must not be null.
unsafe fn required_str<'a>(value: *const c_char, name: &str) -> Result<&'a str> {
    unsafe { optional_str(value) }?
        .ok_or_else(|| PdfError::InvalidArgument(format!("{name} must not be null")))
}

fn owned_string(text: String) -> Result<*mut c_char> {
    CString::new(text)
        .map(CString::into_raw)
        .map_err(|e| PdfError::InvalidArgument(format!("could not return the string: {e}")))
}

fn page_index_from(index: i32) -> Result<usize> {
    usize::try_from(index)
        .map_err(|_| PdfError::InvalidArgument(format!("page index {index} is negative")))
}

fn request_from(zoom: f32, rotation_quarter_turns: i32) -> Result<RenderRequest> {
    if !zoom.is_finite() || zoom <= 0.0 {
        return Err(PdfError::InvalidArgument(format!(
            "zoom must be a positive finite number, got {zoom}"
        )));
    }
    Ok(RenderRequest {
        scale: zoom,
        rotation: Rotation::from_quarter_turns(rotation_quarter_turns),
        ..Default::default()
    })
}

/// Byte order of the caller's buffer. Core Graphics and Android disagree, and the
/// engine converts, so the caller states which one it has rather than the engine
/// guessing from the platform.
fn pixel_order_from(code: i32) -> Result<PixelOrder> {
    match code {
        0 => Ok(PixelOrder::Rgba),
        1 => Ok(PixelOrder::Bgra),
        other => Err(PdfError::InvalidArgument(format!(
            "unknown pixel order {other} (0 = RGBA, 1 = BGRA)"
        ))),
    }
}

/// Packed `0xAARRGGBB`, the form every colour crosses this boundary in.
fn colour_from(argb: i32) -> Color {
    let argb = argb as u32;
    Color {
        a: ((argb >> 24) & 0xff) as u8,
        r: ((argb >> 16) & 0xff) as u8,
        g: ((argb >> 8) & 0xff) as u8,
        b: (argb & 0xff) as u8,
    }
}

/// Decode an optional JSON argument that is allowed to be absent or blank.
///
/// # Safety
/// As [`optional_str`].
unsafe fn optional_json<T: serde::de::DeserializeOwned>(
    value: *const c_char,
    name: &str,
) -> Result<Vec<T>> {
    match unsafe { optional_str(value) }? {
        Some(json) if !json.trim().is_empty() => serde_json::from_str(json)
            .map_err(|e| PdfError::InvalidArgument(format!("could not read {name}: {e}"))),
        _ => Ok(Vec::new()),
    }
}

// --------------------------------------------------------------- lifecycle --

/// Idempotent start-up. Safe to call from an `init()` that may run more than once.
#[no_mangle]
pub extern "C" fn pagify_init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // The unified log, where Console.app and Xcode already look — a device
        // has no stderr anyone reads.
        #[cfg(target_os = "ios")]
        let _ = oslog::OsLogger::new("com.hsilighting.pagify.core")
            .level_filter(log::LevelFilter::Debug)
            .init();

        log::info!("pdf_core {} initialised", env!("CARGO_PKG_VERSION"));
    });
}

/// Point the engine at a PDFium build before the first document is opened.
///
/// iOS has no system PDFium and, at the pinned tag, no static archive either, so
/// the app embeds `libpdfium.dylib` and passes the bundle path it only learns at
/// runtime. Returns [`PAGIFY_OK`] if the path was taken; [`PAGIFY_ERROR`] means
/// PDFium was already bound and the call changed nothing.
///
/// # Safety
/// `path` must be a NUL-terminated string valid for this call.
#[no_mangle]
pub unsafe extern "C" fn pagify_set_pdfium_library_path(path: *const c_char) -> i32 {
    guard(PAGIFY_ERROR, || {
        let path = unsafe { required_str(path, "path") }?;
        if crate::document::pdfium_doc::set_library_path(path.to_string()) {
            Ok(PAGIFY_OK)
        } else {
            Err(PdfError::InvalidArgument(
                "PDFium is already bound; set the library path before opening a document".into(),
            ))
        }
    })
}

/// Crate version. Caller frees with [`pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_version() -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        owned_string(env!("CARGO_PKG_VERSION").to_string())
    })
}

/// Open a document by path. Returns [`PAGIFY_INVALID_HANDLE`] on failure.
///
/// # Safety
/// `path` must be NUL-terminated; `password` NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn pagify_open_document(
    path: *const c_char,
    password: *const c_char,
) -> i64 {
    guard(PAGIFY_INVALID_HANDLE, || {
        let path = unsafe { required_str(path, "path") }?;
        let password = unsafe { optional_str(password) }?;
        registry::insert_with(move || {
            let document = PdfiumDocument::open_path(path, password)?;
            log::debug!("opened {} ({} pages)", path, document.page_count());
            Ok(Box::new(document) as Box<dyn Document>)
        })
    })
}

/// Open a document from a file descriptor.
///
/// **Ownership of `fd` transfers here on every path**, matching the JNI
/// contract: the callee always closes it, including when the open fails.
///
/// # Safety
/// `fd` must be an open descriptor the caller is giving up; `password`
/// NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn pagify_open_document_fd(fd: i32, password: *const c_char) -> i64 {
    guard(PAGIFY_INVALID_HANDLE, || {
        // Adopted before anything else that can fail, so there is no path on
        // which the descriptor is neither owned by a `File` nor still owned by
        // Swift. Safety: the contract above places ownership of `fd` here.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };

        // If this fails, `file` drops here and the descriptor is closed.
        let password = unsafe { optional_str(password) }?;

        registry::insert_with(move || {
            let document = PdfiumDocument::from_file(file, password)?;
            log::debug!("opened fd {} ({} pages)", fd, document.page_count());
            Ok(Box::new(document) as Box<dyn Document>)
        })
    })
}

/// Close a document. Never fails: safe from a `deinit`.
#[no_mangle]
pub extern "C" fn pagify_close_document(handle: i64) -> bool {
    if registry::remove(handle) {
        true
    } else {
        log::warn!("pagify_close_document({handle}) — already closed or never opened");
        false
    }
}

#[no_mangle]
pub extern "C" fn pagify_open_document_count() -> i32 {
    registry::open_count() as i32
}

// ----------------------------------------------------------------- reading --

/// Page count, or -1 on failure — a count the happy path cannot return.
#[no_mangle]
pub extern "C" fn pagify_get_page_count(handle: i64) -> i32 {
    guard(PAGIFY_ERROR, || {
        registry::with_session(handle, |session| Ok(session.document.page_count() as i32))
    })
}

/// Page size in points, written as `[width, height]`.
///
/// # Safety
/// `out_size` must point at space for two `f32`s.
#[no_mangle]
pub unsafe extern "C" fn pagify_get_page_size(
    handle: i64,
    page_index: i32,
    out_size: *mut f32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        if out_size.is_null() {
            return Err(PdfError::InvalidArgument("out_size must not be null".into()));
        }
        let index = page_index_from(page_index)?;
        // Deliberately the no-load path: this is called for every page that
        // scrolls into view, and loading each one would dominate the cost.
        let size = registry::with_session(handle, |session| session.document.page_size(index))?;
        unsafe {
            *out_size = size.width_pt;
            *out_size.add(1) = size.height_pt;
        }
        Ok(PAGIFY_OK)
    })
}

/// The rotation a page carries, in quarter turns. -1 on failure.
#[no_mangle]
pub extern "C" fn pagify_get_page_rotation(handle: i64, page_index: i32) -> i32 {
    guard(PAGIFY_ERROR, || {
        let index = page_index_from(page_index)?;
        registry::with_session(handle, |session| {
            session.document.validate_page_index(index)?;
            match session.document.as_document_mut() {
                Some(doc) => Ok(doc.page_rotation(index)? as i32),
                // Zero for a document that cannot be edited, which is also the
                // right answer: nothing can have rotated it.
                None => Ok(0),
            }
        })
    })
}

/// Every JSON reader below returns a string the caller frees with
/// [`pagify_string_free`], or null on failure.
macro_rules! json_reader {
    ($(#[$meta:meta])* $name:ident, |$session:ident, $index:ident| $body:block) => {
        $(#[$meta])*
        #[no_mangle]
        pub extern "C" fn $name(handle: i64, page_index: i32) -> *mut c_char {
            guard(std::ptr::null_mut(), || {
                let $index = page_index_from(page_index)?;
                let json = registry::with_session(handle, |$session| $body)?;
                owned_string(json)
            })
        }
    };
}

json_reader!(
    /// Text runs with their positions. Coordinates are in points from the page's
    /// top-left, matching [`pagify_get_page_size`].
    pagify_get_text_segments_json,
    |session, index| {
        let page = session.document.page(index)?;
        let segments = page.text_segments()?;
        serde_json::to_string(&segments)
            .map_err(|e| PdfError::Pdfium(format!("could not serialise text segments: {e}")))
    }
);

json_reader!(
    /// A page's text with a box for every character. Costlier than the runs, so
    /// it is a separate call made only when someone actually selects.
    pagify_get_page_characters_json,
    |session, index| {
        let page = session.document.page(index)?;
        let characters = page.characters()?;
        serde_json::to_string(&characters)
            .map_err(|e| PdfError::Pdfium(format!("could not encode characters: {e}")))
    }
);

json_reader!(
    /// Marks on a page, each with PDFium's own index for it. The position in this
    /// list is **not** the annotation's index — a caller erasing by list position
    /// would delete somebody's form field.
    pagify_get_annotations_json,
    |session, index| {
        let marks = session.document.annotations(index)?;
        serde_json::to_string(&marks)
            .map_err(|e| PdfError::Pdfium(format!("could not encode annotations: {e}")))
    }
);

json_reader!(
    /// Every text mark on a page, as the blobs the app stored with them. What
    /// makes words on a saved page a mark again rather than part of the page.
    pagify_get_text_marks_json,
    |session, index| {
        let marks = session.document.text_marks(index)?;
        serde_json::to_string(&marks)
            .map_err(|e| PdfError::Pdfium(format!("could not encode text marks: {e}")))
    }
);

json_reader!(
    /// A page's plain text.
    pagify_get_page_text,
    |session, index| {
        let page = session.document.page(index)?;
        page.text()
    }
);

/// Document metadata as JSON. Caller frees with [`pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_get_metadata_json(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = registry::with_session(handle, |session| {
            let metadata = session.document.metadata()?;
            serde_json::to_string(&metadata)
                .map_err(|e| PdfError::Pdfium(format!("could not serialise metadata: {e}")))
        })?;
        owned_string(json)
    })
}

// ------------------------------------------------------------------ render --

/// Render a page into a caller-supplied 4-bytes-per-pixel buffer.
///
/// The buffer's own dimensions define the render size — `zoom` only identifies
/// the cache entry. That way the caller's rounding, not Rust's, decides the pixel
/// size, and the two can never disagree about how big the target is.
///
/// Returns 1 when the pixels came from cache, 0 when they were rendered, and
/// [`PAGIFY_ERROR`] on failure.
///
/// `pixel_order` is 0 for RGBA or 1 for BGRA. Core Graphics normally wants BGRA
/// premultiplied — **verify it against a known colour rather than trusting this
/// comment**, which is how the Android value was established.
///
/// # Safety
/// `pixels` must be writable for `stride * height` bytes and stay valid for the
/// call. `stride` must be at least `width * 4`.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pagify_render_page_into(
    handle: i64,
    page_index: i32,
    zoom: f32,
    rotation_quarter_turns: i32,
    pixels: *mut u8,
    width: u32,
    height: u32,
    stride: usize,
    pixel_order: i32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        if pixels.is_null() {
            return Err(PdfError::InvalidBitmap("pixels must not be null".into()));
        }
        let index = page_index_from(page_index)?;
        let request = request_from(zoom, rotation_quarter_turns)?;
        let order = pixel_order_from(pixel_order)?;

        // Checked here rather than trusting the caller: a stride narrower than the
        // row would have `RenderTarget` write past the end of the last row.
        let minimum = (width as usize).checked_mul(4).ok_or_else(|| {
            PdfError::InvalidBitmap(format!("width {width} overflows a row"))
        })?;
        if stride < minimum {
            return Err(PdfError::InvalidBitmap(format!(
                "stride {stride} is narrower than a {width}px row"
            )));
        }
        let len = stride.checked_mul(height as usize).ok_or_else(|| {
            PdfError::InvalidBitmap(format!("{width}x{height} overflows a buffer"))
        })?;

        // Safety: the caller's contract above is that this many bytes are
        // writable, and the slice does not outlive the call.
        let buffer = unsafe { std::slice::from_raw_parts_mut(pixels, len) };

        let outcome = registry::with_session(handle, |session| {
            let mut target = RenderTarget::new(width, height, stride, order, buffer)?;
            engine::render_page_into(session, index, &request, &mut target)
        })?;

        Ok(match outcome {
            engine::RenderOutcome::CacheHit => 1,
            engine::RenderOutcome::Rendered => 0,
        })
    })
}

/// Rasterise a page into the cache. Returns 1 if work was done, 0 if it was
/// already cached, [`PAGIFY_ERROR`] on failure.
#[no_mangle]
pub extern "C" fn pagify_prefetch_page(
    handle: i64,
    page_index: i32,
    zoom: f32,
    rotation_quarter_turns: i32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        let index = page_index_from(page_index)?;
        let request = request_from(zoom, rotation_quarter_turns)?;
        let did_work = registry::with_session(handle, |session| {
            engine::prefetch_page(session, index, &request)
        })?;
        Ok(i32::from(did_work))
    })
}

// ----------------------------------------------------------------- editing --

fn edit_state_json(state: engine::EditState) -> Result<*mut c_char> {
    let json = serde_json::to_string(&state)
        .map_err(|e| PdfError::Pdfium(format!("could not encode edit state: {e}")))?;
    owned_string(json)
}

/// Apply one edit, described as a serialised `Command`, and return the resulting
/// edit state as JSON.
///
/// JSON rather than a function per operation, because the command *is* the API:
/// a new operation needs no new export here and no new symbol to keep in step.
///
/// # Safety
/// `command_json` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pagify_execute_command_json(
    handle: i64,
    command_json: *const c_char,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = unsafe { required_str(command_json, "command") }?;
        let command: Command = serde_json::from_str(json).map_err(|e| {
            PdfError::InvalidArgument(format!("could not decode command {json}: {e}"))
        })?;
        let state = registry::with_session(handle, |session| engine::execute(session, command))?;
        edit_state_json(state)
    })
}

/// Reverse the most recent edit.
///
/// An empty history is not an error: the returned state simply still reports
/// `canUndo == false`, and the UI drives its buttons from that. Failing here
/// would turn a double tap into an error dialog.
#[no_mangle]
pub extern "C" fn pagify_undo_edit(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let (_, state) = registry::with_session(handle, engine::undo)?;
        edit_state_json(state)
    })
}

#[no_mangle]
pub extern "C" fn pagify_redo_edit(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let (_, state) = registry::with_session(handle, engine::redo)?;
        edit_state_json(state)
    })
}

#[no_mangle]
pub extern "C" fn pagify_get_edit_state_json(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let state = registry::with_session(handle, |session| Ok(engine::edit_state(session)))?;
        edit_state_json(state)
    })
}

/// Write the document to a descriptor.
///
/// **`fd` must not refer to the file this document was opened from.** PDFium
/// reads objects lazily for a document's whole life, so a save streams *from* the
/// source while writing — pointing both ends at one file truncates the input
/// halfway through and produces a PDF that is neither the old one nor the new
/// one. Write to a scratch file and copy it over, as the Android path does.
///
/// Ownership of `fd` transfers here on every path, matching
/// [`pagify_open_document_fd`].
///
/// `incremental` appends a delta and leaves the original bytes intact, which
/// keeps any existing digital signature valid. A full copy rewrites and compacts
/// the file, and breaks every signature over it.
///
/// # Safety
/// `fd` must be an open, writable descriptor the caller is giving up.
#[no_mangle]
pub unsafe extern "C" fn pagify_save_to_fd(handle: i64, fd: i32, incremental: bool) -> i32 {
    guard(PAGIFY_ERROR, || {
        // Adopted first, so there is no path on which the descriptor is neither
        // owned here nor still owned by Swift.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };
        let mut writer = std::io::BufWriter::new(file);

        registry::with_session(handle, |session| {
            engine::save(session, &mut writer, incremental)
        })?;

        // Flushed explicitly: a `BufWriter` that fails on drop swallows the
        // error, which would report a successful save of a truncated file.
        std::io::Write::flush(&mut writer)?;
        Ok(PAGIFY_OK)
    })
}

/// Write chosen pages out as their own PDF.
///
/// `indices_json` is a JSON array of page indices **in the order they should
/// appear** — "page 3, then page 1" is a thing people ask for, and sorting it
/// quietly hands them a different document.
///
/// Adopts `fd` on every path, as every `..._fd` entry point here does.
///
/// # Safety
/// `indices_json` must be NUL-terminated UTF-8; `fd` an open, writable
/// descriptor the caller is giving up.
#[no_mangle]
pub unsafe extern "C" fn pagify_export_pages_to_fd(
    handle: i64,
    indices_json: *const c_char,
    fd: i32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        let indices: Vec<usize> =
            serde_json::from_str(unsafe { required_str(indices_json, "indices") }?).map_err(|e| {
                PdfError::InvalidArgument(format!("the page list was unreadable: {e}"))
            })?;

        // Adopted first, so on every path from here the descriptor is owned by
        // this call. Safety: the contract above places ownership of `fd` here.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };
        let mut writer = std::io::BufWriter::new(file);

        registry::with_session(handle, |session| {
            let document = session
                .document
                .as_document_mut()
                .ok_or_else(|| PdfError::Pdfium("this document cannot be read from".into()))?;
            let mut extracted = document.extract_pages(&indices)?;
            extracted
                .as_document_mut()
                .ok_or_else(|| PdfError::Pdfium("the extracted pages are not writable".into()))?
                .save_full_copy(&mut writer)
        })?;

        // Flushed explicitly: a `BufWriter` that fails on drop swallows the
        // error, which would report a successful export of a truncated file.
        std::io::Write::flush(&mut writer)?;
        Ok(PAGIFY_OK)
    })
}

/// Bring another document's pages into this one, after `at`.
///
/// Both documents are held at once, through `with_two_sessions` — the registry is
/// one mutex, and the obvious nested borrow deadlocks rather than failing.
///
/// Returns the resulting edit state as JSON. Caller frees.
///
/// # Safety
/// `indices_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn pagify_import_pages(
    handle: i64,
    source_handle: i64,
    indices_json: *const c_char,
    at: i32,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let indices: Vec<usize> =
            serde_json::from_str(unsafe { required_str(indices_json, "indices") }?).map_err(|e| {
                PdfError::InvalidArgument(format!("the page list was unreadable: {e}"))
            })?;
        if indices.is_empty() {
            return Err(PdfError::InvalidArgument(
                "no pages were chosen to import".into(),
            ));
        }
        let at = usize::try_from(at)
            .map_err(|_| PdfError::InvalidArgument(format!("cannot insert at {at}")))?;

        // The pages travel as their own small PDF rather than as a reference to
        // where they came from, so a redo does not depend on the source document
        // still being open.
        let pdf = registry::with_two_sessions(handle, source_handle, |_target, source| {
            let mut extracted = source
                .document
                .as_document_mut()
                .ok_or(PdfError::Unsupported("taking pages from this document"))?
                .extract_pages(&indices)?;
            let mut bytes = Vec::new();
            extracted
                .as_document_mut()
                .ok_or_else(|| PdfError::Pdfium("the chosen pages are not writable".into()))?
                .save_full_copy(&mut bytes)?;
            Ok(bytes)
        })?;

        let state = registry::with_session(handle, |session| {
            engine::execute(session, Command::ImportPages { at, pdf })
        })?;
        edit_state_json(state)
    })
}

/// Write a brand-new blank document straight to a descriptor.
///
/// Takes no handle: there is no document yet, which is the whole point. `fill` is
/// a packed `0xAARRGGBB`, or 0 for paper left the colour paper already is.
///
/// # Safety
/// `fd` must be an open, writable descriptor the caller is giving up.
#[no_mangle]
pub unsafe extern "C" fn pagify_create_blank_document(
    fd: i32,
    pages: i32,
    width_pt: f32,
    height_pt: f32,
    fill: i32,
    ruling: i32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };
        let mut writer = std::io::BufWriter::new(file);

        let pages = usize::try_from(pages.max(0)).unwrap_or(0);
        let size = PageSize {
            width_pt,
            height_pt,
        };
        // Zero means "no fill" rather than transparent black: a colour the reader
        // never chose cannot be told apart from one they did.
        let paint = if fill == 0 { None } else { Some(colour_from(fill)) };

        let bytes = blank_document(pages, size, paint, Ruling::from_code(ruling))?;
        std::io::Write::write_all(&mut writer, &bytes)?;
        std::io::Write::flush(&mut writer)?;
        Ok(PAGIFY_OK)
    })
}

// ----------------------------------------------------------- text and fonts --

/// Hand the engine a font file, under a name the app will ask for later.
///
/// The bytes are copied, so the caller may free them the moment this returns.
///
/// # Safety
/// `name` must be NUL-terminated; `data` must be readable for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn pagify_register_font(
    name: *const c_char,
    data: *const u8,
    len: usize,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        let name = unsafe { required_str(name, "name") }?;
        if data.is_null() {
            return Err(PdfError::InvalidArgument("font data must not be null".into()));
        }
        // Safety: the caller's contract is that `len` bytes are readable.
        let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
        crate::text::register(name, bytes)?;
        Ok(PAGIFY_OK)
    })
}

/// Whether a registered font can draw every character of some text.
///
/// What lets the app pick a font the reader did not: typing Persian into a
/// caption set in Helvetica should produce Persian, not a row of empty boxes.
///
/// # Safety
/// Both arguments must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn pagify_font_covers(name: *const c_char, text: *const c_char) -> bool {
    guard(false, || {
        let name = unsafe { required_str(name, "name") }?;
        let text = unsafe { required_str(text, "text") }?;
        Ok(crate::text::covers(name, text))
    })
}

/// Shape text in a registered font.
///
/// Returns the glyphs in the order they are drawn, as
/// `{"rtl":bool,"glyphs":[{"id","from","to","advance","dx","dy"}]}`. Advances and
/// offsets are fractions of the point size.
///
/// `from`/`to` are byte offsets into the text: which characters this glyph stands
/// for. Send them back with the glyph when saving — they become the ToUnicode,
/// without which the words draw perfectly and cannot be copied.
///
/// # Safety
/// Both arguments must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn pagify_shape_text_json(
    name: *const c_char,
    text: *const c_char,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let name = unsafe { required_str(name, "name") }?;
        let text = unsafe { required_str(text, "text") }?;
        let shaped = crate::text::shape(name, text)?;

        // The characters each glyph stands for, worked out here rather than in
        // Swift: the cluster boundaries are a fact about the shaping, and the app
        // has no way to recover them.
        let mut boundaries: Vec<usize> = shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        let glyphs: Vec<serde_json::Value> = shaped
            .glyphs
            .iter()
            .map(|g| {
                let from = g.cluster as usize;
                let to = boundaries
                    .iter()
                    .find(|&&b| b > from)
                    .copied()
                    .unwrap_or(text.len());
                serde_json::json!({
                    "id": g.id,
                    "from": from,
                    "to": to,
                    "advance": g.advance,
                    "dx": g.offset_x,
                    "dy": g.offset_y,
                })
            })
            .collect();

        owned_string(
            serde_json::json!({ "rtl": shaped.right_to_left, "glyphs": glyphs }).to_string(),
        )
    })
}

// ----------------------------------------------------------------- capture --

/// Re-render one region of a page and hand back an encoded image.
///
/// **Not a screenshot** — the pixels come from the document, so nothing that is
/// not in the document can be in the result. Encoding happens on this side so the
/// uncompressed bitmap never crosses the boundary.
///
/// An empty buffer (`data == null`) means failure; the reason is on
/// [`pagify_last_error_message`]. Free the result with [`pagify_buffer_free`].
///
/// # Safety
/// `format` must be NUL-terminated; `markup_json` NUL-terminated or null.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pagify_capture_region(
    handle: i64,
    page_index: i32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    scale: f32,
    format: *const c_char,
    quality: i32,
    markup_json: *const c_char,
) -> PagifyBuffer {
    guard(PagifyBuffer::empty(), || {
        let index = page_index_from(page_index)?;
        let format = ImageFormat::parse(
            unsafe { required_str(format, "format") }?,
            quality.clamp(1, 100) as u8,
        )?;
        let marks: Vec<Markup> = unsafe { optional_json(markup_json, "markup") }?;
        let request = RegionRequest {
            crop: Rect {
                left,
                top,
                right,
                bottom,
            },
            scale,
            ..Default::default()
        };

        let bytes = registry::with_session(handle, |session| {
            engine::export_region(session.document.as_ref(), index, &request, format, &marks)
        })?;
        Ok(PagifyBuffer::from_vec(bytes))
    })
}

/// Capture what is on screen, across however many pages that turns out to be.
///
/// `tiles_json` holds one entry per page that might contribute — which part of
/// that page, and where it belongs in the picture — because the layout belongs to
/// the app, and working it out again here would be a second copy to keep in step.
///
/// # Safety
/// `tiles_json` and `format` must be NUL-terminated; the other JSON arguments
/// NUL-terminated or null.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pagify_capture_viewport(
    handle: i64,
    tiles_json: *const c_char,
    width: f32,
    height: f32,
    scale: f32,
    background: i32,
    format: *const c_char,
    quality: i32,
    markup_json: *const c_char,
    mask_json: *const c_char,
) -> PagifyBuffer {
    guard(PagifyBuffer::empty(), || {
        let tiles: Vec<Tile> = serde_json::from_str(unsafe { required_str(tiles_json, "tiles") }?)
            .map_err(|e| PdfError::InvalidArgument(format!("could not read the tiles: {e}")))?;
        let format = ImageFormat::parse(
            unsafe { required_str(format, "format") }?,
            quality.clamp(1, 100) as u8,
        )?;
        let marks: Vec<Markup> = unsafe { optional_json(markup_json, "markup") }?;
        let mask: Vec<Point> = unsafe { optional_json(mask_json, "the mask") }?;

        let request = ViewportRequest {
            tiles,
            width,
            height,
            scale,
            background: colour_from(background),
            render_annotations: true,
            render_form_data: true,
        };

        let bytes = registry::with_session(handle, |session| {
            engine::export_viewport(session.document.as_ref(), &request, format, &marks, &mask)
        })?;
        Ok(PagifyBuffer::from_vec(bytes))
    })
}

/// Turn a drawn stroke into a shape, or say it is not one.
///
/// Pure geometry — no document, no handle, no lock — which is why it is safe to
/// call the moment a finger lifts.
///
/// # Safety
/// `points_json` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pagify_recognise_stroke(points_json: *const c_char) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = unsafe { required_str(points_json, "points") }?;
        let points: Vec<Point> = serde_json::from_str(json)
            .map_err(|e| PdfError::InvalidArgument(format!("could not read the stroke: {e}")))?;
        let shape = render::recognise(&points);
        owned_string(
            serde_json::to_string(&shape)
                .map_err(|e| PdfError::Pdfium(format!("could not encode the shape: {e}")))?,
        )
    })
}

// ------------------------------------------------------------------- cache --

#[no_mangle]
pub extern "C" fn pagify_set_cache_budget_bytes(handle: i64, budget_bytes: i64) -> i32 {
    guard(PAGIFY_ERROR, || {
        let budget = usize::try_from(budget_bytes).map_err(|_| {
            PdfError::InvalidArgument(format!("cache budget {budget_bytes} is negative"))
        })?;
        registry::with_session(handle, |session| {
            session.cache.set_budget(budget);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

#[no_mangle]
pub extern "C" fn pagify_clear_cache(handle: i64) -> i32 {
    guard(PAGIFY_ERROR, || {
        registry::with_session(handle, |session| {
            session.cache.clear();
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

#[no_mangle]
pub extern "C" fn pagify_get_cache_stats_json(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let stats = registry::with_session(handle, |session| Ok(session.cache.stats()))?;
        owned_string(
            serde_json::json!({
                "hits": stats.hits,
                "misses": stats.misses,
                "entries": stats.entries,
                "usedBytes": stats.used_bytes,
                "budgetBytes": stats.budget_bytes,
            })
            .to_string(),
        )
    })
}

/// The `onTrimMemory` twin. iOS has no level, so the caller passes one that
/// describes the pressure it saw: at or above 80 documents are closed outright,
/// anything lower only releases cached rasters so the user's document survives.
///
/// `didReceiveMemoryWarning` is the low case; a scene leaving the foreground
/// under pressure is the high one.
#[no_mangle]
pub extern "C" fn pagify_on_trim_memory(level: i32) {
    const TRIM_MEMORY_COMPLETE: i32 = 80;
    if level >= TRIM_MEMORY_COMPLETE {
        log::info!("trim level {level}: closing all documents");
        registry::clear_all();
    } else {
        log::debug!("trim level {level}: releasing cached pages");
        registry::trim_caches();
    }
}
