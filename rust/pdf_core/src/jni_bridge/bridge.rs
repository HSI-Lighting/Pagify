//! The exported JNI functions.
//!
//! Symbol names must match `com.hsilighting.pagify.core.NativeBridge` exactly;
//! changing the Kotlin package means changing every `#[no_mangle]` name here.

use jni::objects::{JClass, JFloatArray, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::command::Command;
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
        // Opened *inside* the registry lock — see `registry::insert_with` for the
        // measurements. Constructing a PDFium document while another thread is
        // opening or closing one lets the two share recycled addresses.
        registry::insert_with(move || {
            let document = PdfiumDocument::open_path(&path, password.as_deref())?;
            log::debug!("opened {} ({} pages)", path, document.page_count());
            Ok(Box::new(document) as Box<dyn Document>)
        })
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

        // Only the PDFium open needs the registry lock; adopting the descriptor
        // above must stay outside it, because it has to happen before anything
        // fallible or the fd would leak on the password path.
        registry::insert_with(move || {
            let document = PdfiumDocument::from_file(file, password.as_deref())?;
            log::debug!("opened fd {} ({} pages)", fd, document.page_count());
            Ok(Box::new(document) as Box<dyn Document>)
        })
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

/// Text runs on a page with their positions, as a JSON array.
///
/// Coordinates are in points from the page's top-left, matching
/// `getPageSize`, so the UI can scale them by the same factor it renders at.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getTextSegmentsJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let index = page_index_from(page_index)?;
        let segments = registry::with_session(handle, |session| {
            let page = session.document.page(index)?;
            page.text_segments()
        })?;

        let json = serde_json::to_string(&segments)
            .map_err(|e| PdfError::Pdfium(format!("could not serialise text segments: {e}")))?;

        Ok(env
            .new_string(json)
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

// ------------------------------------------------------------------- editing --

/// Serialise an [`engine::EditState`] for the return trip.
fn edit_state_json<'local>(env: &JNIEnv<'local>, state: engine::EditState) -> Result<jstring> {
    let json = serde_json::to_string(&state)
        .map_err(|e| PdfError::Pdfium(format!("could not encode edit state: {e}")))?;
    Ok(env
        .new_string(json)
        .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
        .into_raw())
}

/// Apply one edit, described as a serialised [`Command`].
///
/// JSON rather than a function per operation, because the command *is* the API:
/// `Command` already has to serialise for saved scripts, so passing it across the
/// boundary in that form means a new operation needs no new JNI export, no new
/// Kotlin external, and no new symbol name to keep in step.
///
/// @return the resulting [`engine::EditState`] as JSON.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_executeCommandJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    command_json: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = required_string(env, &command_json, "command")?;
        let command: Command = serde_json::from_str(&json).map_err(|e| {
            PdfError::InvalidArgument(format!("could not decode command {json}: {e}"))
        })?;

        let state = registry::with_session(handle, |session| engine::execute(session, command))?;
        edit_state_json(env, state)
    })
}

/// Reverse the most recent edit.
///
/// An empty history is not an error: the returned state simply still reports
/// `canUndo == false`, and the UI drives its buttons from that. Throwing here
/// would turn a double tap into a crash dialog.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_undoEdit<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let (_, state) = registry::with_session(handle, engine::undo)?;
        edit_state_json(env, state)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_redoEdit<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let (_, state) = registry::with_session(handle, engine::redo)?;
        edit_state_json(env, state)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getEditStateJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let state = registry::with_session(handle, |session| Ok(engine::edit_state(session)))?;
        edit_state_json(env, state)
    })
}

/// Write the document to a descriptor.
///
/// **`fd` must not refer to the file this document was opened from.** PDFium reads
/// objects lazily for a document's whole life, so a save streams *from* the source
/// while writing — pointing both ends at one file truncates the input halfway
/// through and produces a PDF that is neither the old one nor the new one. Kotlin
/// writes to a scratch file and copies it over afterwards; see `PdfDocument.saveTo`.
///
/// Ownership of `fd` transfers here on every path, matching `openDocumentFd`.
///
/// `incremental` appends a delta and leaves the original bytes intact, which keeps
/// any existing digital signature valid. A full copy rewrites and compacts the
/// file, and breaks every signature over it.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_saveToFd(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    fd: jint,
    incremental: jboolean,
) {
    guard(&mut env, (), |_| {
        // Adopted first so there is no path on which the descriptor is neither
        // owned by a `File` here nor still owned by Kotlin.
        // Safety: the contract above places ownership of `fd` with this call.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };
        let mut writer = std::io::BufWriter::new(file);

        registry::with_session(handle, |session| {
            engine::save(session, &mut writer, incremental == JNI_TRUE)
        })?;

        // Flushed explicitly: a `BufWriter` that fails on drop swallows the error,
        // which would report a successful save of a truncated file.
        std::io::Write::flush(&mut writer)?;
        Ok(())
    })
}

/// The rotation a page currently carries, in quarter turns.
///
/// Needed because `Command::SetPageRotation` is absolute rather than relative: an
/// undo record has to restore the angle the page actually had, and a relative
/// command could not describe that. The UI turns pages by a quarter at a time, so
/// it has to read the current value to work out what to ask for.
///
/// Zero for a document that cannot be edited, which is also the right answer —
/// nothing can have rotated it.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getPageRotation(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    page_index: jint,
) -> jint {
    guard(&mut env, 0, |_| {
        let index = page_index_from(page_index)?;
        registry::with_session(handle, |session| {
            session.document.validate_page_index(index)?;
            match session.document.as_document_mut() {
                Some(doc) => Ok(doc.page_rotation(index)? as jint),
                None => Ok(0),
            }
        })
    })
}

/// Marks already on a page, each with PDFium's own index for it.
///
/// The index matters more than it looks: a page can carry form widgets and links
/// this engine does not model, and those are skipped rather than guessed at. So
/// the position in this list is *not* the annotation's index, and a caller that
/// erases by list position would delete somebody's form field.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getAnnotationsJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let index = page_index_from(page_index)?;
        let json = registry::with_session(handle, |session| {
            let marks = session.document.annotations(index)?;
            serde_json::to_string(&marks)
                .map_err(|e| PdfError::Pdfium(format!("could not encode annotations: {e}")))
        })?;

        Ok(env
            .new_string(json)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}
