//! The 3D viewer's surface to Kotlin.
//!
//! Deliberately small: a handle to an open model, four ways to move the view,
//! and one call that fills a bitmap. Everything else — parsing, tessellating,
//! shading — stays in Rust, so the Android side is a gesture detector and an
//! `Image`, and the iOS side would be the same.
//!
//! **A separate registry from the PDF one.** Nothing here can reach a document
//! session and nothing there can reach a model, so a bug in this feature cannot
//! disturb the reader that people actually use.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jfloat, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::error::{PdfError, Result};
use crate::jni_bridge::android_bitmap::LockedPixels;
use crate::jni_bridge::{guard, required_string};
use crate::step::session::{self, ModelSession};

type Sessions = Mutex<HashMap<i64, ModelSession>>;

fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn with_model<T>(handle: i64, work: impl FnOnce(&mut ModelSession) -> T) -> Result<T> {
    let mut open = sessions()
        .lock()
        .map_err(|_| PdfError::InvalidArgument("the model registry is poisoned".into()))?;
    let model = open
        .get_mut(&handle)
        .ok_or_else(|| PdfError::InvalidArgument("no such model".into()))?;
    Ok(work(model))
}

/// Open a STEP file. Returns a handle, or throws with the reason.
///
/// **The reason is thrown, not swallowed.** A file this cannot show is the
/// common case — half of real files contain something unsupported — so what
/// comes back has to be a sentence the screen can put in front of somebody,
/// not a null that leaves them guessing.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_openModel<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> jlong {
    guard(&mut env, 0, |env| {
        let path = required_string(env, &path, "path")?;
        let bytes = std::fs::read(&path)
            .map_err(|error| PdfError::InvalidArgument(format!("the file could not be read: {error}")))?;

        let model = session::open(&path, &bytes)
            .map_err(|error| PdfError::InvalidArgument(error.message()))?;

        let handle = next_handle();
        sessions()
            .lock()
            .map_err(|_| PdfError::InvalidArgument("the model registry is poisoned".into()))?
            .insert(handle, model);
        Ok(handle)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_closeModel(
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

/// What the model is, for the screen to describe — including what was skipped.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_modelSummaryJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let summary = with_model(handle, |model| model.summary_json())?;
        Ok(env
            .new_string(summary)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

/// Draw the model into a bitmap.
///
/// The same handover `renderPageInto` uses, which is what lets a 3D view be an
/// ordinary `Image` on both platforms rather than a new kind of surface.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_renderModelInto<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    bitmap: JObject<'local>,
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

        let canvas = with_model(handle, |model| model.draw(width, height))?;

        // Row by row, because a bitmap may pad its rows and the canvas does
        // not. Copying the whole buffer would shear the picture on any device
        // whose stride is not exactly four bytes a pixel.
        let target = locked.as_mut_slice();
        let row_bytes = (width as usize) * 4;
        for row in 0..(height as usize) {
            let from = row * row_bytes;
            let to = row * stride;
            if to + row_bytes <= target.len() && from + row_bytes <= canvas.pixels.len() {
                target[to..to + row_bytes]
                    .copy_from_slice(&canvas.pixels[from..from + row_bytes]);
            }
        }

        Ok(JNI_TRUE)
    })
}

/// Turn the part, in fractions of the view rather than pixels.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_orbitModel(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    across: jfloat,
    down: jfloat,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_model(handle, |model| model.orbit(across as f64, down as f64))?;
        Ok(JNI_TRUE)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_panModel(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    across: jfloat,
    down: jfloat,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_model(handle, |model| model.pan(across as f64, down as f64))?;
        Ok(JNI_TRUE)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_zoomModel(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    by: jfloat,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_model(handle, |model| model.zoom(by as f64))?;
        Ok(JNI_TRUE)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_fitModel(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        with_model(handle, |model| model.fit())?;
        Ok(JNI_TRUE)
    })
}

/// How many models are open, for a test to prove closing works.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_StepBridge_openModelCount(
    mut env: JNIEnv,
    _class: JClass,
) -> jlong {
    guard(&mut env, 0, |_| {
        Ok(sessions().lock().map(|open| open.len() as i64).unwrap_or(0))
    })
}
