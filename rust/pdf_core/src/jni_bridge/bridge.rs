//! The exported JNI functions.
//!
//! Symbol names must match `com.hsilighting.pagify.core.NativeBridge` exactly;
//! changing the Kotlin package means changing every `#[no_mangle]` name here.

use jni::objects::{JByteArray, JClass, JFloatArray, JObject, JString};
use jni::sys::{jboolean, jbyteArray, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::command::Command;
use crate::document::pdfium_doc::PdfiumDocument;
use crate::document::blank::{blank_document, Ruling};
use crate::document::{
    Color, Document, PageSize, Point, Rect, RegionRequest, RenderRequest, Rotation,
};
use crate::engine;
use crate::error::{PdfError, Result};
use crate::jni_bridge::android_bitmap::LockedPixels;
use crate::jni_bridge::{guard, optional_string, required_string};
use crate::registry;
use crate::render::{self, ImageFormat, Markup, PixelOrder, RenderTarget, Tile, ViewportRequest};

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
        let did_work = registry::with_session(handle, |session| {
            engine::prefetch_page(session, index, &request)
        })?;
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

/// Write a brand-new blank document straight to a file descriptor.
///
/// Takes no handle: there is no document yet, which is the whole point. The
/// descriptor is one Kotlin opened on a destination the reader chose, so the file
/// exists in their storage from the moment it exists at all, and is then opened
/// by the ordinary path like any other file.
///
/// `fill` is an ARGB colour, or 0 for paper left the colour paper already is.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_createBlankDocument(
    mut env: JNIEnv,
    _class: JClass,
    fd: jint,
    pages: jint,
    width_pt: jfloat,
    height_pt: jfloat,
    fill: jint,
    ruling: jint,
) {
    guard(&mut env, (), |_| {
        // Adopted first, as in `saveToFd`: on every path from here the descriptor
        // is owned by this call.
        // Safety: the contract above places ownership of `fd` with this call.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };
        let mut writer = std::io::BufWriter::new(file);

        let pages = usize::try_from(pages.max(0)).unwrap_or(0);
        let size = PageSize { width_pt, height_pt };
        // Zero means "no fill" rather than transparent black: a colour the reader
        // never chose cannot be told apart from one they did, and white paper is
        // sent as no rectangle at all.
        let paint = if fill == 0 {
            None
        } else {
            let argb = fill as u32;
            Some(Color {
                a: ((argb >> 24) & 0xff) as u8,
                r: ((argb >> 16) & 0xff) as u8,
                g: ((argb >> 8) & 0xff) as u8,
                b: (argb & 0xff) as u8,
            })
        };

        let bytes = blank_document(pages, size, paint, Ruling::from_code(ruling))?;
        std::io::Write::write_all(&mut writer, &bytes)?;
        // Flushed explicitly: a `BufWriter` that fails on drop swallows the error,
        // which would report a written file that is truncated.
        std::io::Write::flush(&mut writer)?;
        Ok(())
    })
}

/// Hand the engine a font file, under a name the app will ask for later.
///
/// Registered once at startup rather than passed with every caption: a font file
/// is most of a megabyte and a caption is a few dozen bytes.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_registerFont<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    data: JByteArray<'local>,
) {
    guard(&mut env, (), |env| {
        let name = required_string(env, &name, "name")?;
        let bytes = env
            .convert_byte_array(&data)
            .map_err(|e| PdfError::InvalidArgument(format!("could not read the font: {e}")))?;
        crate::text::register(&name, bytes)
    })
}

/// Whether a registered font can draw every character of some text.
///
/// What lets the app pick a font the reader did not: typing Persian into a
/// caption set in Helvetica should produce Persian, not a row of empty boxes.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_fontCovers<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    text: JString<'local>,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |env| {
        let name = required_string(env, &name, "name")?;
        let text = required_string(env, &text, "text")?;
        Ok(if crate::text::covers(&name, &text) {
            JNI_TRUE
        } else {
            JNI_FALSE
        })
    })
}

/// Shape text in a registered font.
///
/// Returns the glyphs in the order they are drawn, left to right, as
/// `{"rtl":bool,"glyphs":[{"id":u32,"from":u32,"to":u32,"advance":f32,"dx":f32,"dy":f32}]}`.
/// Advances and offsets are fractions of the point size, so the app scales them
/// by whatever size the reader chose without asking again.
///
/// `from`/`to` are byte offsets into the text: which characters this glyph stands
/// for. The app sends them back with the glyph when it saves, and they become the
/// ToUnicode — without which the words draw perfectly and cannot be copied.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_shapeTextJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    text: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let name = required_string(env, &name, "name")?;
        let text = required_string(env, &text, "text")?;
        let shaped = crate::text::shape(&name, &text)?;

        // The characters each glyph stands for, worked out here rather than in
        // Kotlin: the cluster boundaries are a fact about the shaping, and the
        // app has no way to recover them.
        let mut boundaries: Vec<usize> =
            shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
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

        let payload = serde_json::json!({
            "rtl": shaped.right_to_left,
            "glyphs": glyphs,
        })
        .to_string();

        env.new_string(payload)
            .map(|s| s.into_raw())
            .map_err(|e| PdfError::InvalidArgument(format!("could not return the shaping: {e}")))
    })
}

/// Write chosen pages of a document out as a new PDF.
///
/// `indices` is a JSON array of page numbers, taken in the order given: "pages 3,
/// 1 and 2" is a thing somebody can ask for, and sorting the list quietly would
/// hand them a different document.
///
/// Takes ownership of `fd` exactly as `saveToFd` does.
///
/// **Marks made this session are not in the document yet.** The caller commits
/// them first — the same rule that "Save a copy" learned the hard way, when it
/// silently produced files with every stroke missing.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_exportPagesToFd<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    indices_json: JString<'local>,
    fd: jint,
) {
    guard(&mut env, (), |env| {
        let indices: Vec<usize> = serde_json::from_str(&required_string(
            env,
            &indices_json,
            "indices",
        )?)
        .map_err(|e| PdfError::InvalidArgument(format!("the page list was unreadable: {e}")))?;

        // Adopted first, so on every path from here the descriptor is owned by
        // this call.
        // Safety: the contract above places ownership of `fd` with this call.
        let file = unsafe { PdfiumDocument::adopt_fd(fd)? };
        let mut writer = std::io::BufWriter::new(file);

        registry::with_session(handle, |session| {
            let document = session
                .document
                .as_document_mut()
                .ok_or_else(|| PdfError::Pdfium("this document cannot be read from".into()))?;
            let extracted = document.extract_pages(&indices)?;
            let mut extracted = extracted;
            extracted
                .as_document_mut()
                .ok_or_else(|| PdfError::Pdfium("the extracted pages are not writable".into()))?
                .save_full_copy(&mut writer)
        })?;

        // Flushed explicitly: a `BufWriter` that fails on drop swallows the error,
        // which would report a successful export of a truncated file.
        std::io::Write::flush(&mut writer)?;
        Ok(())
    })
}

/// Put pages from another open document into this one.
///
/// Both documents have to be open at once, and the registry is behind a single
/// mutex — so this goes through `with_two_sessions` rather than nesting two
/// borrows, which would deadlock rather than fail.
///
/// The chosen pages are extracted into a small PDF first and the *command* holds
/// those bytes. That is what makes the import redoable: redo re-executes against
/// the document as it now stands, and a command pointing at the source file could
/// not run once that file was closed.
///
/// Returns the edit state, as every other mutating entry does.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_importPages<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    source_handle: jlong,
    indices_json: JString<'local>,
    at: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let indices: Vec<usize> = serde_json::from_str(&required_string(
            env,
            &indices_json,
            "indices",
        )?)
        .map_err(|e| PdfError::InvalidArgument(format!("the page list was unreadable: {e}")))?;
        if indices.is_empty() {
            return Err(PdfError::InvalidArgument(
                "no pages were chosen to import".into(),
            ));
        }
        let at = usize::try_from(at)
            .map_err(|_| PdfError::InvalidArgument(format!("cannot insert at {at}")))?;

        let pdf = registry::with_two_sessions(handle, source_handle, |_target, source| {
            // Through `as_document_mut` because `extract_pages` lives on
            // `DocumentMut`. Nothing about the source changes — the extraction
            // builds a new document — but the trait it sits on is the mutable one.
            let extracted = source
                .document
                .as_document_mut()
                .ok_or_else(|| PdfError::Unsupported("taking pages from this document"))?
                .extract_pages(&indices)?;
            let mut extracted = extracted;
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
        edit_state_json(env, state)
    })
}

/// Write a contact as a vCard.
///
/// The card arrives as JSON and the timestamp as an RFC 3339 string, because the
/// clock belongs to the platform: Android has the user's locale and time zone
/// and the engine does not. The date given here is written into the file as
/// `REV`, which is the whole point of the feature — an exported contact that
/// still says when it was exported.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_contactToVCard<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    card_json: JString<'local>,
    exported_at: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = required_string(env, &card_json, "card")?;
        let exported_at = required_string(env, &exported_at, "exportedAt")?;
        let card: crate::contacts::BusinessCard = serde_json::from_str(&json)
            .map_err(|e| PdfError::InvalidArgument(format!("could not decode the card: {e}")))?;

        let vcard = crate::contacts::to_vcard(&card, &exported_at);
        env.new_string(vcard)
            .map(|s| s.into_raw())
            .map_err(|e| PdfError::InvalidArgument(format!("could not return the vCard: {e}")))
    })
}

/// Several contacts in one file.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_contactsToVCard<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cards_json: JString<'local>,
    exported_at: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = required_string(env, &cards_json, "cards")?;
        let exported_at = required_string(env, &exported_at, "exportedAt")?;
        let cards: Vec<crate::contacts::BusinessCard> = serde_json::from_str(&json)
            .map_err(|e| PdfError::InvalidArgument(format!("could not decode the cards: {e}")))?;

        let vcard = crate::contacts::to_vcards(&cards, &exported_at);
        env.new_string(vcard)
            .map(|s| s.into_raw())
            .map_err(|e| PdfError::InvalidArgument(format!("could not return the vCard: {e}")))
    })
}

/// Read a vCard into a contact, or return null when the text is not one.
///
/// This is the QR path. A business card carrying a QR that encodes a vCard needs
/// no detection, no rectification and no OCR — the data is exact rather than
/// recognised.
///
/// **Null is a real answer, not a failure.** Most QR codes on business cards hold
/// a URL rather than a vCard, and the caller has to be able to tell the two
/// apart so it can fall through to reading the card by eye instead of showing a
/// blank contact.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_contactFromVCard<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let text = required_string(env, &text, "text")?;
        let Some(card) = crate::contacts::from_vcard(&text) else {
            return Ok(std::ptr::null_mut());
        };

        let json = serde_json::to_string(&card)
            .map_err(|e| PdfError::InvalidArgument(format!("could not encode the card: {e}")))?;
        env.new_string(json)
            .map(|s| s.into_raw())
            .map_err(|e| PdfError::InvalidArgument(format!("could not return the card: {e}")))
    })
}

/// Read the cards in a photograph, given the text recognised on it.
///
/// The printed path, and the reason the QR path returning null is survivable.
/// Recognition itself belongs to the platform — ML Kit here, Vision on iOS —
/// but deciding which line is the name and which is the company does not, so
/// only the recognised boxes cross this boundary.
///
/// Returns a JSON **array**. One photograph can hold several cards — six emptied
/// out of a pocket after an event is the case that asked for this — and
/// `split_cards` decides where one ends and the next begins before any of them is
/// read.
///
/// The segments arrive in the photograph's pixel space and no detection has run
/// yet, so each card is measured from its own text: see
/// [`RecognisedCard::around_text`], which is also what a caller that *has* found
/// the cards' rectangles should stop using.
///
/// Never null, and possibly empty. Whether an empty result is worth reporting is
/// the caller's question — it is the one that knows whether a QR already answered
/// it.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_parsePhotographedCard<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    segments_json: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = required_string(env, &segments_json, "segments")?;
        let segments: Vec<crate::contacts::parse::TextSegment> = serde_json::from_str(&json)
            .map_err(|e| {
                PdfError::InvalidArgument(format!("could not decode the recognised text: {e}"))
            })?;

        // An **array** of cards, because one photograph can hold several. Six
        // cards emptied out of a pocket after an event is the case this exists
        // for; one card is the same path with one element.
        let cards: Vec<_> = crate::contacts::parse::split_cards(segments)
            .iter()
            .map(crate::contacts::parse::parse_card)
            .collect();

        let json = serde_json::to_string(&cards)
            .map_err(|e| PdfError::InvalidArgument(format!("could not encode the cards: {e}")))?;
        env.new_string(json)
            .map(|s| s.into_raw())
            .map_err(|e| PdfError::InvalidArgument(format!("could not return the cards: {e}")))
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

/// Every text mark on a page, as the blobs the app stored with them.
///
/// Text is page content rather than an annotation, so it does not appear in
/// `getAnnotationsJson` and has no index to erase by. These come back instead:
/// each is the app's own description of one caption, put there when it was
/// written, and what makes words on a saved page a mark again rather than part
/// of the page.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getTextMarksJson<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let index = page_index_from(page_index)?;
        let json = registry::with_session(handle, |session| {
            let marks = session.document.text_marks(index)?;
            serde_json::to_string(&marks)
                .map_err(|e| PdfError::Pdfium(format!("could not encode text marks: {e}")))
        })?;

        Ok(env
            .new_string(json)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

// ----------------------------------------------------------------- capture --

/// Re-render one region of a page and hand back an encoded image.
///
/// **Not a screenshot** — decision 4.8. The pixels come from the document, so
/// nothing that is not in the document can be in the result: no notification, no
/// dialog of ours, no status bar. That is a property of where the pixels come
/// from rather than something filtered out afterwards, which is why this exists
/// instead of a `MediaProjection` capture.
///
/// The crop is in page points with a top-left origin, the same space annotations
/// use. `scale` is the export resolution and is independent of the on-screen
/// zoom; it is lowered if it would breach the render ceiling.
///
/// Encoding happens on this side so the uncompressed bitmap never crosses the
/// boundary: a 4× capture is tens of megabytes as pixels and a fraction of that
/// as a PNG.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_captureRegion<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
    left: jfloat,
    top: jfloat,
    right: jfloat,
    bottom: jfloat,
    scale: jfloat,
    format: JString<'local>,
    quality: jint,
    markup_json: JString<'local>,
) -> jbyteArray {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let index = page_index_from(page_index)?;
        let format = ImageFormat::parse(
            &required_string(env, &format, "format")?,
            quality.clamp(1, 100) as u8,
        )?;
        let marks: Vec<Markup> = match optional_string(env, &markup_json)? {
            Some(json) if !json.trim().is_empty() => serde_json::from_str(&json)
                .map_err(|e| PdfError::InvalidArgument(format!("could not read markup: {e}")))?,
            _ => Vec::new(),
        };
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

        Ok(env
            .byte_array_from_slice(&bytes)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate the capture: {e}")))?
            .into_raw())
    })
}

/// Capture what is on screen, across however many pages that turns out to be.
///
/// The reader lays pages in a column, so a box dragged around something
/// interesting very often crosses a join. `tilesJson` holds one entry per page
/// that might contribute — which part of that page, and where it belongs in the
/// picture — because the layout belongs to the app, and working it out again here
/// would be a second copy to keep in step.
///
/// Still not a screenshot: every pixel is rendered from the document, so the gaps
/// between pages come out as `background` rather than as whatever the app happened
/// to be drawing there, and nothing floating above the reader can appear at all.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_captureViewport<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    tiles_json: JString<'local>,
    width: jfloat,
    height: jfloat,
    scale: jfloat,
    background: jint,
    format: JString<'local>,
    quality: jint,
    markup_json: JString<'local>,
    mask_json: JString<'local>,
) -> jbyteArray {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let tiles: Vec<Tile> =
            serde_json::from_str(&required_string(env, &tiles_json, "tiles")?)
                .map_err(|e| PdfError::InvalidArgument(format!("could not read the tiles: {e}")))?;
        let format = ImageFormat::parse(
            &required_string(env, &format, "format")?,
            quality.clamp(1, 100) as u8,
        )?;
        let marks: Vec<Markup> = match optional_string(env, &markup_json)? {
            Some(json) if !json.trim().is_empty() => serde_json::from_str(&json)
                .map_err(|e| PdfError::InvalidArgument(format!("could not read markup: {e}")))?,
            _ => Vec::new(),
        };

        let mask: Vec<Point> = match optional_string(env, &mask_json)? {
            Some(json) if !json.trim().is_empty() => serde_json::from_str(&json)
                .map_err(|e| PdfError::InvalidArgument(format!("could not read the mask: {e}")))?,
            _ => Vec::new(),
        };

        let request = ViewportRequest {
            tiles,
            width,
            height,
            scale,
            // Packed `0xAARRGGBB`, the form the app stores every other colour in.
            background: Color {
                a: (background >> 24) as u8,
                r: (background >> 16) as u8,
                g: (background >> 8) as u8,
                b: background as u8,
            },
            render_annotations: true,
            render_form_data: true,
        };

        let bytes = registry::with_session(handle, |session| {
            engine::export_viewport(session.document.as_ref(), &request, format, &marks, &mask)
        })?;

        Ok(env
            .byte_array_from_slice(&bytes)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate the capture: {e}")))?
            .into_raw())
    })
}

/// Turn a drawn stroke into a shape, or say it is not one.
///
/// Takes the stroke's points as JSON and returns a [`crate::render::Shape`] the
/// app can commit as markup. Pure geometry — no document, no handle, no lock —
/// which is why it is safe to call the moment a finger lifts.
///
/// It declines far more readily than it snaps: a squiggle that stays a squiggle
/// costs nothing, and a squiggle silently turned into a circle costs the user
/// their drawing.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_recogniseStroke<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    points_json: JString<'local>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = required_string(env, &points_json, "points")?;
        let points: Vec<Point> = serde_json::from_str(&json)
            .map_err(|e| PdfError::InvalidArgument(format!("could not read the stroke: {e}")))?;

        let shape = render::recognise(&points);
        let encoded = serde_json::to_string(&shape)
            .map_err(|e| PdfError::Pdfium(format!("could not encode the shape: {e}")))?;

        Ok(env
            .new_string(encoded)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}

/// A page's text with a box for every character, as JSON.
///
/// What selection needs and [`Java_com_hsilighting_pagify_core_NativeBridge_getTextSegmentsJson`]
/// cannot give: a run is a whole line, so a selection built from runs can only
/// start and end at a line. Characters let it start and end where the finger is.
///
/// Costlier than the runs — a dense page is thousands of boxes — so it is a
/// separate call, made only when someone actually selects.
#[no_mangle]
pub extern "system" fn Java_com_hsilighting_pagify_core_NativeBridge_getPageCharactersJson<
    'local,
>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page_index: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let index = page_index_from(page_index)?;
        let json = registry::with_session(handle, |session| {
            let page = session.document.page(index)?;
            let characters = page.characters()?;
            serde_json::to_string(&characters)
                .map_err(|e| PdfError::Pdfium(format!("could not encode characters: {e}")))
        })?;

        Ok(env
            .new_string(json)
            .map_err(|e| PdfError::Pdfium(format!("could not allocate Java string: {e}")))?
            .into_raw())
    })
}
