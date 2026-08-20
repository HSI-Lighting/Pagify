package com.hsilighting.pagify.core

import android.graphics.Bitmap

/**
 * The one and only door into the Rust engine.
 *
 * Every symbol here has a matching `#[no_mangle]` export in
 * `rust/pdf_core/src/jni_bridge/bridge.rs`; the JNI symbol names embed this
 * class's package, so moving or renaming this file means editing the Rust side
 * too.
 *
 * Nothing above [PdfDocument] should call these directly — the native layer
 * deals in raw handles and has no idea about lifecycles.
 */
internal object NativeBridge {

    const val INVALID_HANDLE: Long = -1L

    init {
        // libpdfium must be resident before pdf_core's first `dlopen("libpdfium.so")`.
        // pdf_core does not list it as a DT_NEEDED (it binds dynamically), so
        // without this the first document open would fail with a library error
        // rather than anything that points at the real cause.
        System.loadLibrary("pdfium")
        System.loadLibrary("pdf_core")
        nativeInit()
    }

    /** Installs the Rust logger. Idempotent. */
    private external fun nativeInit()

    external fun nativeVersion(): String

    // ------------------------------------------------------------ lifecycle --

    /** @return a handle, or [INVALID_HANDLE]. Throws on failure. */
    @Throws(PdfException::class)
    external fun openDocument(path: String, password: String?): Long

    /**
     * Opens from a file descriptor taken with `ParcelFileDescriptor.detachFd()`.
     *
     * This is the path used for `content://` URIs. Ownership of [fd] passes to
     * native code, which closes it in [closeDocument] — passing a `getFd()` result
     * here would let the JVM close it under an open document.
     */
    @Throws(PdfException::class)
    external fun openDocumentFd(fd: Int, password: String?): Long

    /** Never throws. @return false if the handle was already closed. */
    external fun closeDocument(handle: Long): Boolean

    /** Number of documents the native side still holds open — used by leak tests. */
    external fun openDocumentCount(): Int

    // -------------------------------------------------------------- document --

    @Throws(PdfException::class)
    external fun getPageCount(handle: Long): Int

    /** Document metadata as JSON; see [PdfMetadata.fromJson]. */
    @Throws(PdfException::class)
    external fun getMetadataJson(handle: Long): String

    /** `[widthPoints, heightPoints]` at the page's natural size. */
    @Throws(PdfException::class)
    external fun getPageSize(handle: Long, pageIndex: Int): FloatArray

    @Throws(PdfException::class)
    external fun getPageText(handle: Long, pageIndex: Int): String

    /**
     * Text runs with their positions, as JSON; see [TextSegment.listFromJson].
     *
     * Points from the page's top-left, matching [getPageSize], so the same scale
     * used to render a page also maps these onto it.
     */
    @Throws(PdfException::class)
    external fun getTextSegmentsJson(handle: Long, pageIndex: Int): String

    // ---------------------------------------------------------------- render --

    /**
     * Renders a page directly into [bitmap]'s pixels — no intermediate buffer, no
     * copy.
     *
     * [bitmap] must be `ARGB_8888` and mutable. **Its dimensions decide the render
     * size**; [zoom] only identifies the cache entry, so Kotlin's rounding is the
     * single source of truth about how many pixels a page occupies.
     *
     * @return true when the pixels came from the native cache rather than a fresh
     *   rasterisation. Useful for instrumentation; ignore it otherwise.
     */
    @Throws(PdfException::class)
    external fun renderPageInto(
        handle: Long,
        pageIndex: Int,
        zoom: Float,
        rotationQuarterTurns: Int,
        bitmap: Bitmap,
    ): Boolean

    /**
     * Rasterises a page into the native cache. Call off the main thread for pages
     * adjacent to the visible one so a swipe resolves to a memcpy.
     *
     * @return true if it did work; false if that page was already cached.
     */
    @Throws(PdfException::class)
    external fun prefetchPage(
        handle: Long,
        pageIndex: Int,
        zoom: Float,
        rotationQuarterTurns: Int,
    ): Boolean

    // ------------------------------------------------------------------ edit --

    /**
     * Applies one [PdfCommand], given as its JSON form.
     *
     * JSON rather than one external per operation because the command *is* the
     * API: adding an operation then needs no new native symbol and no new
     * declaration here, only a new [PdfCommand] variant on each side.
     *
     * @return the resulting [EditState] as JSON.
     */
    @Throws(PdfException::class)
    external fun executeCommandJson(handle: Long, commandJson: String): String

    /**
     * Reverses the most recent edit.
     *
     * An empty history is not an error — the returned state simply still reports
     * `canUndo == false`. Callers drive their buttons from that rather than from a
     * thrown exception, so a double tap is a no-op.
     */
    @Throws(PdfException::class)
    external fun undoEdit(handle: Long): String

    @Throws(PdfException::class)
    external fun redoEdit(handle: Long): String

    @Throws(PdfException::class)
    external fun getEditStateJson(handle: Long): String

    /**
     * Marks already in the document on this page, as JSON.
     *
     * Each carries PDFium's own index, which is what an erase addresses. A page
     * can hold form widgets and links the engine does not model; those are absent
     * from the list, so a position in it is not an index.
     */
    @Throws(PdfException::class)
    external fun getAnnotationsJson(handle: Long, pageIndex: Int): String

    /**
     * The rotation a page currently carries, in quarter turns.
     *
     * [PdfCommand.SetPageRotation] is absolute rather than relative, because an
     * undo record has to restore the angle a page actually had. A UI that turns by
     * a quarter at a time therefore has to read the current value first.
     */
    @Throws(PdfException::class)
    external fun getPageRotation(handle: Long, pageIndex: Int): Int

    /**
     * Writes the document to [fd], taking ownership of it exactly as
     * [openDocumentFd] does.
     *
     * **[fd] must not point at the file this document was opened from.** PDFium
     * reads objects lazily for the document's whole life, so a save reads from the
     * source while writing; aiming both ends at one file truncates the input
     * mid-save. [PdfDocument.saveTo] is the path that gets this right.
     */
    @Throws(PdfException::class)
    external fun saveToFd(handle: Long, fd: Int, incremental: Boolean)

    // ----------------------------------------------------------------- cache --

    @Throws(PdfException::class)
    external fun setCacheBudgetBytes(handle: Long, budgetBytes: Long)

    @Throws(PdfException::class)
    external fun clearCache(handle: Long)

    @Throws(PdfException::class)
    external fun getCacheStatsJson(handle: Long): String

    /** Mirrors `ComponentCallbacks2.onTrimMemory`. Never throws. */
    external fun onTrimMemory(level: Int)

    // --------------------------------------------------------------- capture --

    /**
     * Re-renders whatever was framed on screen and returns it encoded.
     *
     * **This is not a screenshot, and that is the whole design.** The pixels come
     * from the document, so nothing that is not in the document can appear in the
     * result — no notification, no dialog of ours, no status bar. Those are
     * consequences of never involving the screen rather than things filtered out
     * afterwards; see roadmap decision 4.8 for the alternatives and why each was
     * rejected.
     *
     * A capture is **not one page**. The reader stacks pages in a column, so a box
     * dragged around something interesting routinely takes the bottom of one page,
     * the gap below it and the top of the next. [tilesJson] carries one entry per
     * page that contributes — which part of it, and where that part belongs — as
     * the app is the only side that knows where a page sits on a screen.
     *
     * [width] and [height] are in capture units, and [scale] multiplies them for
     * the export: the framing comes from the screen, the resolution does not. The
     * scale is lowered if it would breach the render ceiling, so a large area at 4×
     * comes back as the sharpest picture that fits rather than as a failure.
     *
     * Encoding happens natively so the uncompressed bitmap never crosses this
     * boundary: a 4× capture is tens of megabytes as pixels and a fraction of that
     * as a PNG.
     *
     * @param background `0xAARRGGBB`, shown wherever no page reaches.
     * @param format `"png"` or `"jpeg"`.
     * @param quality 1–100, ignored for PNG.
     * @param markupJson marks to draw on it, in capture units. `[]` for none.
     * @param maskJson a ring to keep, in capture units; everything outside it is
     *   painted over with [background]. `[]` captures the whole rectangle.
     */
    @Throws(PdfException::class)
    external fun captureViewport(
        handle: Long,
        tilesJson: String,
        width: Float,
        height: Float,
        scale: Float,
        background: Int,
        format: String,
        quality: Int,
        markupJson: String,
        maskJson: String,
    ): ByteArray

    /**
     * A page's text with a box for every character, as JSON.
     *
     * What selecting text needs and [getTextSegmentsJson] cannot give: a run is a
     * whole line, so a selection built from runs can only begin and end at a
     * line. Characters let it begin and end where the finger is.
     *
     * Costlier than the runs — a dense page is thousands of boxes — so it is a
     * separate call, made only when someone actually selects.
     */
    @Throws(PdfException::class)
    external fun getPageCharactersJson(handle: Long, pageIndex: Int): String

    /**
     * Turn a drawn stroke into a shape, or say it is not one.
     *
     * Pure geometry — no document, no handle, no lock — so it is safe to call the
     * moment a finger lifts, on whatever thread the gesture is on.
     *
     * It declines far more readily than it snaps. That is deliberate: a squiggle
     * that stays a squiggle costs nothing, and a squiggle silently turned into a
     * circle costs the user their drawing.
     *
     * @param pointsJson `[{"x":..,"y":..}, …]` in page points.
     * @return one shape in the markup wire form. `freehand` means "not a shape".
     */
    @Throws(PdfException::class)
    external fun recogniseStroke(pointsJson: String): String
}
