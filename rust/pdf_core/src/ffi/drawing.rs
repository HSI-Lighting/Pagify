//! The C ABI surface for the DXF/DWG drawing viewer — iOS's counterpart of
//! Android's `jni_bridge::drawing_bridge`. Same shape as [`super::model`]:
//! its own handle registry, [`super::guard`] around every call, failures on
//! [`super::pagify_last_error_message`].
//!
//! DWG and DXF are not two engines behind this one bridge — `drawing::dwg`
//! converts into the exact same `Drawing` type `drawing::dxf` parses
//! directly (see `drawing::session::open`, which chooses the reader by
//! extension), so everything past `open` — render, pan, zoom, measure,
//! layers — is one code path regardless of which file came in.

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{guard, owned_string, optional_str, required_str, PAGIFY_ERROR, PAGIFY_INVALID_HANDLE, PAGIFY_OK};
use crate::drawing::session::DrawingSession;
use crate::error::{PdfError, Result};

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

fn sessions() -> &'static Mutex<HashMap<i64, DrawingSession>> {
    static SESSIONS: std::sync::OnceLock<Mutex<HashMap<i64, DrawingSession>>> =
        std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> MutexGuard<'static, HashMap<i64, DrawingSession>> {
    sessions().lock().unwrap_or_else(PoisonError::into_inner)
}

fn with_session<T>(handle: i64, f: impl FnOnce(&mut DrawingSession) -> Result<T>) -> Result<T> {
    let mut guard = lock();
    let session = guard.get_mut(&handle).ok_or(PdfError::InvalidHandle(handle))?;
    f(session)
}

/// Open a `.dxf` or `.dwg` file by path (the extension picks the reader —
/// see `drawing::session::open`'s own doc for why that's deliberate rather
/// than sniffed), returning a handle, or [`PAGIFY_INVALID_HANDLE`] with a
/// message on [`super::pagify_last_error_message`].
///
/// `font_name`, if not null, is a font already registered with
/// [`super::pagify_register_font`] — drawings carry no usable font of their
/// own (SHX/Windows fonts never travel with CAD files), so text is drawn
/// with the app's own substituted font. Best-effort: a missing or unreadable
/// font here does not fail the open, matching Android's own "retried on
/// first draw if the font wasn't loaded yet."
///
/// # Safety
/// `path` must be a valid, NUL-terminated UTF-8 string. `font_name` must be
/// null or likewise valid, for the call.
#[no_mangle]
pub unsafe extern "C" fn pagify_open_drawing(
    path: *const c_char,
    font_name: *const c_char,
) -> i64 {
    guard(PAGIFY_INVALID_HANDLE, || {
        let path = unsafe { required_str(path, "path") }?;
        let font_name = unsafe { optional_str(font_name) }?;

        let mut session = crate::drawing::session::open(std::path::Path::new(path))
            .map_err(|e| PdfError::InvalidArgument(e.message()))?;

        if let Some(name) = font_name {
            if let Ok(bytes) = crate::text::font_data(name) {
                session.use_font(bytes);
            }
        }

        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        lock().insert(handle, session);
        Ok(handle)
    })
}

/// Close a drawing, freeing its geometry. Returns whether a live drawing was
/// actually closed — a double close is tolerated, matching
/// [`super::pagify_close_document`]'s own convention.
#[no_mangle]
pub extern "C" fn pagify_close_drawing(handle: i64) -> bool {
    if lock().remove(&handle).is_some() {
        true
    } else {
        log::warn!("pagify_close_drawing({handle}) — already closed or never opened");
        false
    }
}

/// Shape/layer counts, size, declared units and what was skipped, as JSON —
/// the same shape `DrawingSession::summary_json` already produces for
/// Android.
///
/// **Caller owns the returned string**, released with
/// [`super::pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_drawing_summary_json(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = with_session(handle, |session| Ok(session.summary_json()))?;
        owned_string(json)
    })
}

/// The layer list — name, visibility, colour — as JSON.
///
/// **Caller owns the returned string**, released with
/// [`super::pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_drawing_layers_json(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = with_session(handle, |session| Ok(session.layers_json()))?;
        owned_string(json)
    })
}

/// Show or hide one layer by its index in [`pagify_drawing_layers_json`]'s
/// own list.
#[no_mangle]
pub extern "C" fn pagify_show_drawing_layer(handle: i64, at: i64, visible: bool) -> i32 {
    guard(PAGIFY_ERROR, || {
        let at = usize::try_from(at)
            .map_err(|_| PdfError::InvalidArgument(format!("layer index {at} is negative")))?;
        with_session(handle, |session| {
            session.show_layer(at, visible);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Move the view centre. `across`/`down` are fractions of the view (a
/// one-finger drag); `width`/`height` are the current viewport size in
/// pixels, needed because the pan distance in drawing-space depends on the
/// current zoom scale.
#[no_mangle]
pub extern "C" fn pagify_pan_drawing(
    handle: i64,
    across: f32,
    down: f32,
    width: u32,
    height: u32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.pan(across as f64, down as f64, width, height);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Zoom about a screen point — `at_x`/`at_y` (pixels, e.g. a pinch's
/// midpoint) stay fixed on screen; `by` is a multiplier, greater than 1
/// moves closer, matching a pinch's `apart / lastApart` ratio.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn pagify_zoom_drawing(
    handle: i64,
    by: f32,
    at_x: f32,
    at_y: f32,
    width: u32,
    height: u32,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.zoom_about(by as f64, at_x as f64, at_y as f64, width, height);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Reset the view to frame the whole drawing for a given viewport size.
#[no_mangle]
pub extern "C" fn pagify_fit_drawing(handle: i64, width: u32, height: u32) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.fit(width, height);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Place a measure point at a screen position, snapped to nearby geometry
/// (endpoint, intersection, midpoint or centre, in that priority — see
/// `drawing::measure`), and return the running measurement as JSON: what was
/// snapped to, and once two points are placed, the distance in the
/// drawing's own units (never converted to metric — half of these files
/// never declare real-world units, and a wrong conversion is worse than
/// none). A third tap starts the measurement over rather than chaining.
///
/// **Caller owns the returned string**, released with
/// [`super::pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_measure_drawing_at(
    handle: i64,
    at_x: f32,
    at_y: f32,
    width: u32,
    height: u32,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = with_session(handle, |session| {
            session.measure_at(at_x as f64, at_y as f64, width, height);
            Ok(session.measure_json())
        })?;
        owned_string(json)
    })
}

/// Clear the current measurement (e.g. when the tool is turned off).
#[no_mangle]
pub extern "C" fn pagify_clear_drawing_measure(handle: i64) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.clear_measure();
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Rasterise the current view into a caller-owned RGBA8 buffer — the same
/// contract as [`super::pagify_render_page_into`] and
/// [`super::model::pagify_render_model_into`]. `tiny-skia` (already a
/// dependency for capture markup) fills a tightly-packed `Pixmap`
/// internally; this copies it out row by row so a caller-chosen `stride` is
/// always respected. Hatching is not a separate feature to wire up here —
/// `drawing::raster::draw` expands a hatch's boundary and pattern into
/// strokes at draw time, scaled to the current zoom, so it is simply part of
/// what this call already draws.
///
/// # Safety
/// `pixels` must be writable for `stride * height` bytes and stay valid for
/// the call. `stride` must be at least `width * 4`.
#[no_mangle]
pub unsafe extern "C" fn pagify_render_drawing_into(
    handle: i64,
    width: u32,
    height: u32,
    pixels: *mut u8,
    stride: usize,
) -> i32 {
    guard(PAGIFY_ERROR, || {
        if pixels.is_null() {
            return Err(PdfError::InvalidBitmap("pixels must not be null".into()));
        }
        let row_bytes = (width as usize).checked_mul(4).ok_or_else(|| {
            PdfError::InvalidBitmap(format!("width {width} overflows a row"))
        })?;
        if stride < row_bytes {
            return Err(PdfError::InvalidBitmap(format!(
                "stride {stride} is narrower than a {width}px row"
            )));
        }
        let len = stride.checked_mul(height as usize).ok_or_else(|| {
            PdfError::InvalidBitmap(format!("{width}x{height} overflows a buffer"))
        })?;
        // Safety: the caller's contract above is that this many bytes are
        // writable, and the slice does not outlive the call.
        let out = unsafe { std::slice::from_raw_parts_mut(pixels, len) };

        with_session(handle, |session| {
            let pixmap = session.draw(width, height);
            let data = pixmap.data();
            for row in 0..height as usize {
                let src = &data[row * row_bytes..(row + 1) * row_bytes];
                let dst_start = row * stride;
                out[dst_start..dst_start + row_bytes].copy_from_slice(src);
            }
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}
