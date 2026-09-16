//! The C ABI surface for the STEP (3D model) viewer — iOS's counterpart of
//! Android's `jni_bridge::step_bridge` (a JNI bridge, not a C one; the calling
//! convention doesn't port, only the shape of the API does). Same three rules
//! as the rest of [`super`]: panics are caught by [`super::guard`], failures
//! report through [`super::pagify_last_error_message`], and every handle here
//! is this module's own — never [`crate::registry`]'s, which is PDF-specific.
//!
//! A session owns one [`crate::step::session::ModelSession`] — a single
//! flattened mesh plus a trackball camera, never a part tree (STEP assemblies
//! are flattened to world space during tessellation; see `step::assembly`).
//! Every camera call is followed by the caller asking for a fresh frame with
//! [`pagify_render_model_into`] — nothing here redraws on its own.

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{guard, owned_string, required_str, PAGIFY_ERROR, PAGIFY_INVALID_HANDLE, PAGIFY_OK};
use crate::error::{PdfError, Result};
use crate::step::session::ModelSession;

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

fn sessions() -> &'static Mutex<HashMap<i64, ModelSession>> {
    static SESSIONS: std::sync::OnceLock<Mutex<HashMap<i64, ModelSession>>> =
        std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A panic while a model was borrowed poisons the lock; recovering it is the
/// right call — the alternative is every open model bricked until restart,
/// over one malformed file. Matches [`crate::registry`]'s own reasoning.
fn lock() -> MutexGuard<'static, HashMap<i64, ModelSession>> {
    sessions().lock().unwrap_or_else(PoisonError::into_inner)
}

fn with_session<T>(handle: i64, f: impl FnOnce(&mut ModelSession) -> Result<T>) -> Result<T> {
    let mut guard = lock();
    let session = guard.get_mut(&handle).ok_or(PdfError::InvalidHandle(handle))?;
    f(session)
}

/// Open a STEP file (`.step`/`.stp`/`.p21`) by path, returning a handle, or
/// [`PAGIFY_INVALID_HANDLE`] with a message on [`super::pagify_last_error_message`]
/// — most often the audit's own refusal sentence (too many faces, no solid
/// found, mostly freeform surfaces) rather than a generic failure.
///
/// # Safety
/// `path` must be a valid, NUL-terminated UTF-8 string for the call.
#[no_mangle]
pub unsafe extern "C" fn pagify_open_model(path: *const c_char) -> i64 {
    guard(PAGIFY_INVALID_HANDLE, || {
        let path = unsafe { required_str(path, "path") }?;
        let bytes = std::fs::read(path)
            .map_err(|e| PdfError::InvalidArgument(format!("could not read {path}: {e}")))?;
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);

        let session = crate::step::session::open(name, &bytes).map_err(|e| {
            PdfError::InvalidArgument(e.message())
        })?;

        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        lock().insert(handle, session);
        Ok(handle)
    })
}

/// Close a model, freeing its mesh. Returns whether a live model was actually
/// closed — a double close is tolerated, matching the JNI side and
/// [`super::pagify_close_document`]'s own convention.
#[no_mangle]
pub extern "C" fn pagify_close_model(handle: i64) -> bool {
    if lock().remove(&handle).is_some() {
        true
    } else {
        log::warn!("pagify_close_model({handle}) — already closed or never opened");
        false
    }
}

/// Triangle/face counts, bounding-box size, surface-type breakdown and what
/// was skipped, as the same JSON shape `ModelSession::summary_json` already
/// produces for Android — one encoding, read by both platforms' details
/// panels.
///
/// **Caller owns the returned string** and must release it with
/// [`super::pagify_string_free`].
#[no_mangle]
pub extern "C" fn pagify_model_summary_json(handle: i64) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let json = with_session(handle, |session| Ok(session.summary_json()))?;
        owned_string(json)
    })
}

/// Rotate the trackball camera. `across`/`down` are fractions of the view —
/// matching Android's own `orbit`, so a one-finger drag maps the same way on
/// both platforms: `across = moved.x / viewWidth`, `down = moved.y / viewWidth`
/// (the *width* on both axes deliberately, so a non-square view doesn't skew
/// one axis against the other).
#[no_mangle]
pub extern "C" fn pagify_orbit_model(handle: i64, across: f32, down: f32) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.orbit(across as f64, down as f64);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Slide the camera's look-at target. `across`/`down` are fractions of the
/// view, matching a two-finger drag.
#[no_mangle]
pub extern "C" fn pagify_pan_model(handle: i64, across: f32, down: f32) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.pan(across as f64, down as f64);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Zoom the camera. `by` is a multiplier — greater than 1 moves closer,
/// matching a pinch's `apart / lastApart` ratio.
#[no_mangle]
pub extern "C" fn pagify_zoom_model(handle: i64, by: f32) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.zoom(by as f64);
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Reset the camera to frame the whole model.
#[no_mangle]
pub extern "C" fn pagify_fit_model(handle: i64) -> i32 {
    guard(PAGIFY_ERROR, || {
        with_session(handle, |session| {
            session.fit();
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}

/// Rasterise the current camera view into a caller-owned RGBA8 buffer —
/// exactly [`super::pagify_render_page_into`]'s own contract, so both call
/// sites on the Swift side share one "buffer to `CGImage`" helper. The
/// software rasteriser (depth-buffered, flat-shaded, no GPU — see
/// `step::raster`'s own module doc) writes into a tightly-packed `Canvas`
/// internally; this copies it out row by row so a caller-chosen `stride`
/// (e.g. one `CGContext` picks for alignment) is always respected.
///
/// # Safety
/// `pixels` must be writable for `stride * height` bytes and stay valid for
/// the call. `stride` must be at least `width * 4`.
#[no_mangle]
pub unsafe extern "C" fn pagify_render_model_into(
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
            let canvas = session.draw(width, height);
            for row in 0..height as usize {
                let src = &canvas.pixels[row * row_bytes..(row + 1) * row_bytes];
                let dst_start = row * stride;
                out[dst_start..dst_start + row_bytes].copy_from_slice(src);
            }
            Ok(())
        })?;
        Ok(PAGIFY_OK)
    })
}
