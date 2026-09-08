//! The drawing viewer's surface to Kotlin.
//!
//! The same shape as [`super::step_bridge`]: a handle to an open drawing, ways
//! to move the view, and one call that fills a bitmap. Reading, stroking and
//! the formats themselves stay in Rust, so the Android side is a gesture
//! detector and an `Image`.
//!
//! **A third registry, separate from the model one and the PDF one.** Nothing
//! here can reach a document session or a model, so a fault in a format this
//! app has only just learned to read cannot disturb the reader people rely on.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::drawing::session::{self, DrawingSession};
use crate::error::{PdfError, Result};
use crate::jni_bridge::android_bitmap::LockedPixels;
use crate::jni_bridge::{guard, required_string};

type Sessions = Mutex<HashMap<i64, DrawingSession>>;

fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn with_drawing<T>(handle: i64, work: impl FnOnce(&mut DrawingSession) -> T) -> Result<T> {
    let mut open = sessions()
        .lock()
        .map_err(|_| PdfError::InvalidArgument("the drawing registry is poisoned".into()))?;
    let drawing = open
        .get_mut(&handle)
        .ok_or_else(|| PdfError::InvalidArgument("no such drawing".into()))?;
    Ok(work(drawing))
}

/// Open a `.dwg` or `.dxf`. Returns a handle, or throws with the reason.
///
/// The reason is thrown rather than swallowed, as the model viewer does it: a
/// drawing this cannot show has to come back as a sentence somebody can read,
/// not as a zero that leaves them guessing.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_openDrawing<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> jlong {
    guard(&mut env, 0, |env| {
        let path = required_string(env, &path, "path")?;
        let drawing = session::open(std::path::Path::new(&path))
            .map_err(|error| PdfError::InvalidArgument(error.message()))?;

        let handle = next_handle();
        sessions()
            .lock()
            .map_err(|_| PdfError::InvalidArgument("the drawing registry is poisoned".into()))?
            .insert(handle, drawing);
        Ok(handle)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_closeDrawing(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        let removed = sessions()
            .lock()
            .map(|mut open| open.remove(&handle).is_some())
            .unwrap_or(false);
        Ok(if removed { JNI_TRUE } else { JNI_FALSE })
    })
}

/// What the drawing holds, for the screen to describe — including what is not
/// shown.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_drawingSummaryJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let summary = with_drawing(handle, |drawing| drawing.summary_json())?;
        Ok(env
            .new_string(summary)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

/// The layers, so the screen can offer to turn them off.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_drawingLayersJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let layers = with_drawing(handle, |drawing| drawing.layers_json())?;
        Ok(env
            .new_string(layers)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

/// Draw the sheet into a bitmap.
///
/// The same handover `renderPageInto` and `renderModelInto` use, which is what
/// lets a drawing be an ordinary `Image` on both platforms.
///
/// **The multiple is not optional.** A sheet's scale is pixels per drawing
/// unit, so a larger bitmap shows more of the drawing rather than the same view
/// in more detail; without being told how much larger it is, a capture cuts the
/// wrong region out of a wider picture.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_renderDrawingInto<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    bitmap: JObject<'local>,
    // How much larger this bitmap is than the one on screen: one for an
    // ordinary frame, two for a capture drawn at twice the detail.
    by: jfloat,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |env| {
        // Safety: `bitmap` is a live local reference for this call, and the
        // lock is released before it is dropped.
        let mut locked = unsafe { LockedPixels::lock(env, &bitmap)? };
        let (width, height, stride) = (
            locked.info.width,
            locked.info.height,
            locked.info.stride as usize,
        );

        let sheet =
            with_drawing(handle, |drawing| drawing.draw_scaled(width, height, by as f64))?;

        // Row by row, because a bitmap may pad its rows and the sheet does not.
        // Copying the whole buffer would shear the picture on any device whose
        // stride is not exactly four bytes a pixel.
        let source = sheet.data();
        let target = locked.as_mut_slice();
        let row_bytes = (width as usize) * 4;
        for row in 0..(height as usize) {
            let from = row * row_bytes;
            let to = row * stride;
            if to + row_bytes <= target.len() && from + row_bytes <= source.len() {
                target[to..to + row_bytes].copy_from_slice(&source[from..from + row_bytes]);
            }
        }

        Ok(JNI_TRUE)
    })
}

/// Slide the sheet. Both distances are fractions of the view, not pixels.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_panDrawing(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    across: jfloat,
    down: jfloat,
    width: jint,
    height: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_drawing(handle, |drawing| {
            drawing.pan(across as f64, down as f64, width.max(1) as u32, height.max(1) as u32)
        })?;
        Ok(JNI_TRUE)
    })
}

/// Zoom about a point on the screen. Above one is closer.
///
/// Takes where the fingers are, because a pinch that zooms about the middle of
/// the view has to be dragged back afterwards every single time.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_zoomDrawing(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    by: jfloat,
    at_x: jfloat,
    at_y: jfloat,
    width: jint,
    height: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_drawing(handle, |drawing| {
            drawing.zoom_about(
                by as f64,
                at_x as f64,
                at_y as f64,
                width.max(1) as u32,
                height.max(1) as u32,
            )
        })?;
        Ok(JNI_TRUE)
    })
}

/// Give the sheet a font to draw its text with.
///
/// **A drawing brings no usable font of its own.** Its text names an SHX stroke
/// font or a Windows typeface, neither of which travels with the file, so every
/// viewer substitutes one. This hands over the app's own — the same bytes the
/// PDF side already registered, looked up by the name it registered them under,
/// so nothing is read or parsed twice.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_useDrawingFont<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    name: JString<'local>,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |env| {
        let name = required_string(env, &name, "name")?;
        let bytes = crate::text::font_data(&name)?;
        with_drawing(handle, |drawing| drawing.use_font(bytes))?;
        Ok(JNI_TRUE)
    })
}

/// Show the whole sheet.
///
/// Takes the view's size, unlike the model viewer's fit: a camera in space can
/// frame a solid without knowing the aspect it will be drawn at, and a flat
/// sheet cannot.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_fitDrawing(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    width: jint,
    height: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_drawing(handle, |drawing| {
            drawing.fit(width.max(1) as u32, height.max(1) as u32)
        })?;
        Ok(JNI_TRUE)
    })
}

/// Turn one layer on or off.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_showDrawingLayer(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    at: jint,
    visible: jboolean,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_drawing(handle, |drawing| {
            drawing.show_layer(at.max(0) as usize, visible != JNI_FALSE)
        })?;
        Ok(JNI_TRUE)
    })
}

/// How many drawings are open, for a test to prove closing works.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_DrawingBridge_openDrawingCount(
    mut env: JNIEnv,
    _class: JClass,
) -> jlong {
    guard(&mut env, 0, |_| {
        Ok(sessions().lock().map(|open| open.len() as i64).unwrap_or(0))
    })
}
