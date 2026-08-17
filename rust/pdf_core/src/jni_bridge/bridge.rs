//! The exported JNI functions.
//!
//! Symbol names must match `com.hsilighting.pagify.core.NativeBridge` exactly;
//! changing the Kotlin package means changing every `#[no_mangle]` name here.

use jni::objects::{JClass, JFloatArray, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::document::pdfium_doc::PdfiumDocument;
use crate::document::{Document, RenderRequest, Rotation};
use crate::engine;
use crate::error::{PdfError, Result};
use crate::jni_bridge::android_bitmap::LockedPixels;
use crate::jni_bridge::{guard, optional_string, required_string};
use crate::registry;
use crate::render::{PixelOrder, RenderTarget};

const INVALID_HANDLE: jlong = -1;

fn request_from(zoom: jfloat, rotation_quarter_turns: jint) -> Result<RenderRequest> {
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

fn page_index_from(index: jint) -> Result<usize> {
    usize::try_from(index)
        .map_err(|_| PdfError::InvalidArgument(format!("page index {index} is negative")))
}

// ---------------------------------------------------------------- lifecycle --

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_nativeInit(
    _env: JNIEnv,
    _class: JClass,
) {
    // Idempotent: the Kotlin side calls this from a static initialiser, but a
    // second call after process restart in the same VM must not double-install.
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("PagifyCore"),
        );
        log::info!("pdf_core {} initialised", env!("CARGO_PKG_VERSION"));
    });
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_openDocument<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
    password: JString<'local>,
) -> jlong {
    guard(&mut env, INVALID_HANDLE, |env| {
        let path = required_string(env, &path, "path")?;
        let password = optional_string(env, &password)?;
        let document = PdfiumDocument::open_path(&path, password.as_deref())?;
        log::debug!("opened {} ({} pages)", path, document.page_count());
        Ok(registry::insert(Box::new(document)))
    })
}

/// Open from a descriptor obtained via `ParcelFileDescriptor.detachFd()`.
///
/// **Ownership of `fd` transfers to native code unconditionally.** On success the
/// document holds it until closed; on *any* failure it is closed here. The caller
/// must never close it itself — doing so would risk closing a descriptor number
/// the OS has since handed to something else.
///
/// Callers must use `detachFd`, not `getFd`, or the JVM will close it out from
/// under an open document.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_openDocumentFd<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    fd: jint,
    password: JString<'local>,
) -> jlong {
    guard(&mut env, INVALID_HANDLE, |env| {
        // Adopted before anything else that can fail, so there is no path on which
        // the descriptor is neither owned by a `File` nor still owned by Kotlin.
        // Safety: the contract above places ownership of `fd` with this call.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };

        // If this fails, `file` drops here and the descriptor is closed.
        let password = optional_string(env, &password)?;

        let document = PdfiumDocument::from_file(file, password.as_deref())?;
        log::debug!("opened fd {} ({} pages)", fd, document.page_count());
        Ok(registry::insert(Box::new(document)))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_closeDocument(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    // No `guard`: closing must never throw, so it stays safe to call from a
    // `finally` block or a ViewModel's `onCleared`.
    if registry::remove(handle) {
        JNI_TRUE
    } else {
        log::warn!("closeDocument({handle}) — already closed or never opened");
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_openDocumentCount(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    registry::open_count() as jint
}

// ----------------------------------------------------------------- document --

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getPageCount(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    guard(&mut env, 0, |_| {
        registry::with_session(handle, |session| Ok(session.document.page_count() as jint))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getMetadataJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = registry::with_session(handle, |session| {
            let metadata = session.document.metadata()?;
            serde_json::to_string(&metadata)
                .map_err(|e| PdfError::Pdfium(format!("could not serialise metadata: {e}")))
        })?;

        Ok(env
            .new_string(json)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

/// Page size in points as a two-element array `[width, height]`.
///
/// A float array rather than a small Java object keeps the JNI surface free of
/// reflection, per the security note in the architecture doc.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getPageSize<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
) -> JFloatArray<'local> {
    let fallback = JObject::null().into();
    guard(&mut env, fallback, |env| {
        let index = page_index_from(page_index)?;
        // Deliberately the no-load path: this is called for every page that
        // scrolls into view, and loading each one would dominate the cost.
        let size = registry::with_session(handle, |session| session.document.page_size(index))?;

        let array = env
            .new_float_array(2)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate float array: {e}")))?;
        env.set_float_array_region(&array, 0, &[size.width_pt, size.height_pt])
            .map_err(|e| PdfError::Pdfium(format!("could not fill float array: {e}")))?;
        Ok(array)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getPageText<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let index = page_index_from(page_index)?;
        let text = registry::with_session(handle, |session| {
            let page = session.document.page(index)?;
            page.text()
        })?;

        Ok(env
            .new_string(text)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

// ------------------------------------------------------------------ render --

/// Render a page into a caller-supplied `ARGB_8888` bitmap.
///
/// The bitmap's own dimensions define the render size — `zoom` only identifies
/// the cache entry. That way Kotlin's rounding, not Rust's, decides the pixel
/// size, and the two can never disagree about how big the target is.
///
/// Returns `true` when the pixels came from cache.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_renderPageInto<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
    zoom: jfloat,
    rotation_quarter_turns: jint,
    bitmap: JObject<'local>,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |env| {
        let index = page_index_from(page_index)?;
        let request = request_from(zoom, rotation_quarter_turns)?;

        // Safety: `bitmap` is a live local reference for the duration of this call,
        // and the guard unlocks before it is dropped.
        let mut locked = unsafe { LockedPixels::lock(env, &bitmap)? };
        let (width, height, stride) = (
            locked.info.width,
            locked.info.height,
            locked.info.stride as usize,
        );

        let outcome = registry::with_session(handle, |session| {
            let mut target = RenderTarget::new(
                width,
                height,
                stride,
                PixelOrder::Rgba,
                locked.as_mut_slice(),
            )?;
            engine::render_page_into(session, index, &request, &mut target)
        })?;

        Ok(match outcome {
            engine::RenderOutcome::CacheHit => JNI_TRUE,
            engine::RenderOutcome::Rendered => JNI_FALSE,
        })
    })
}

/// Rasterise a page into the cache. Intended for a background dispatcher, so the
/// adjacent pages are ready before the user swipes to them.
///
/// Returns `true` if work was actually done (`false` means it was already cached).
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_prefetchPage(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    page_index: jint,
    zoom: jfloat,
    rotation_quarter_turns: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        let index = page_index_from(page_index)?;
        let request = request_from(zoom, rotation_quarter_turns)?;
        let did_work =
            registry::with_session(handle, |session| engine::prefetch_page(session, index, &request))?;
        Ok(if did_work { JNI_TRUE } else { JNI_FALSE })
    })
}

// ------------------------------------------------------------------- cache --

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_setCacheBudgetBytes(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    budget_bytes: jlong,
) {
    guard(&mut env, (), |_| {
        let budget = usize::try_from(budget_bytes).map_err(|_| {
            PdfError::InvalidArgument(format!("cache budget {budget_bytes} is negative"))
        })?;
        registry::with_session(handle, |session| {
            session.cache.set_budget(budget);
            Ok(())
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_clearCache(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        registry::with_session(handle, |session| {
            session.cache.clear();
            Ok(())
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getCacheStatsJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let stats = registry::with_session(handle, |session| Ok(session.cache.stats()))?;
        let json = serde_json::json!({
            "hits": stats.hits,
            "misses": stats.misses,
            "entries": stats.entries,
            "usedBytes": stats.used_bytes,
            "budgetBytes": stats.budget_bytes,
        })
        .to_string();

        Ok(env
            .new_string(json)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

/// Mirrors `ComponentCallbacks2.onTrimMemory`. Levels at or above
/// `TRIM_MEMORY_COMPLETE` (80) close documents outright; anything lower only
/// releases cached rasters so the user's document survives.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_onTrimMemory(
    _env: JNIEnv,
    _class: JClass,
    level: jint,
) {
    const TRIM_MEMORY_COMPLETE: jint = 80;
    if level >= TRIM_MEMORY_COMPLETE {
        log::info!("trim level {level}: closing all documents");
        registry::clear_all();
    } else {
        log::debug!("trim level {level}: releasing cached pages");
        registry::trim_caches();
    }
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_nativeVersion<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        Ok(env
            .new_string(env!("CARGO_PKG_VERSION"))
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}
