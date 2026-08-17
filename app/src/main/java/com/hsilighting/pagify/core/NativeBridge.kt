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

    // ----------------------------------------------------------------- cache --

    @Throws(PdfException::class)
    external fun setCacheBudgetBytes(handle: Long, budgetBytes: Long)

    @Throws(PdfException::class)
    external fun clearCache(handle: Long)

    @Throws(PdfException::class)
    external fun getCacheStatsJson(handle: Long): String

    /** Mirrors `ComponentCallbacks2.onTrimMemory`. Never throws. */
    external fun onTrimMemory(level: Int)
}
