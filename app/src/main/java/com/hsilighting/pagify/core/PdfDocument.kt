package com.hsilighting.pagify.core

import android.content.ContentResolver
import android.graphics.Bitmap
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.util.Log
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * An open PDF, owning its native handle.
 *
 * [Closeable] rather than a finalizer: the native side holds a file descriptor
 * and potentially tens of megabytes of cached rasters, and waiting for the GC to
 * notice would let a user who opens several documents in a row run the app out of
 * descriptors.
 *
 * Instances are safe to use from any thread — the native registry serialises
 * access — but calls made after [close] throw [IllegalStateException] rather than
 * reaching native code with a dangling handle.
 */
class PdfDocument private constructor(
    private val handle: Long,
    /** Human-readable origin, used as the title fallback. */
    val sourceName: String,
) : Closeable {

    private val closed = AtomicBoolean(false)

    val pageCount: Int by lazy { NativeBridge.getPageCount(requireOpen()) }

    val metadata: PdfMetadata by lazy {
        PdfMetadata.fromJson(NativeBridge.getMetadataJson(requireOpen()))
    }

    /** Page size in PostScript points, at 100% zoom. */
    fun pageSize(pageIndex: Int): PageSize {
        val raw = NativeBridge.getPageSize(requireOpen(), pageIndex)
        return PageSize(widthPoints = raw[0], heightPoints = raw[1])
    }

    fun pageText(pageIndex: Int): String = NativeBridge.getPageText(requireOpen(), pageIndex)

    /** Positioned text runs for selection and highlighting. See [TextSegment]. */
    fun textSegments(pageIndex: Int): List<TextSegment> =
        TextSegment.listFromJson(NativeBridge.getTextSegmentsJson(requireOpen(), pageIndex))

    /**
     * Renders into [bitmap], whose dimensions decide the output size.
     *
     * @return true if served from the native cache.
     */
    fun renderPageInto(
        pageIndex: Int,
        bitmap: Bitmap,
        zoom: Float,
        rotationQuarterTurns: Int = 0,
    ): Boolean {
        require(bitmap.config == Bitmap.Config.ARGB_8888) {
            "target bitmap must be ARGB_8888, was ${bitmap.config}"
        }
        require(bitmap.isMutable) { "target bitmap must be mutable" }
        return NativeBridge.renderPageInto(
            requireOpen(), pageIndex, zoom, rotationQuarterTurns, bitmap,
        )
    }

    /** @return true if it rendered; false if that page was already cached. */
    fun prefetchPage(pageIndex: Int, zoom: Float, rotationQuarterTurns: Int = 0): Boolean =
        NativeBridge.prefetchPage(requireOpen(), pageIndex, zoom, rotationQuarterTurns)

    fun setCacheBudgetBytes(budgetBytes: Long) =
        NativeBridge.setCacheBudgetBytes(requireOpen(), budgetBytes)

    fun clearCache() = NativeBridge.clearCache(requireOpen())

    fun cacheStats(): PdfCacheStats =
        PdfCacheStats.fromJson(NativeBridge.getCacheStatsJson(requireOpen()))

    /** Idempotent, and safe to call from a `finally` block. */
    override fun close() {
        if (closed.compareAndSet(false, true)) {
            NativeBridge.closeDocument(handle)
        }
    }

    val isClosed: Boolean get() = closed.get()

    private fun requireOpen(): Long {
        check(!closed.get()) { "document '$sourceName' has already been closed" }
        return handle
    }

    companion object {
        private const val TAG = "PdfDocument"

        fun openFile(path: String, password: String? = null): PdfDocument {
            val handle = NativeBridge.openDocument(path, password)
            check(handle != NativeBridge.INVALID_HANDLE) { "native open returned no handle" }
            return PdfDocument(handle, path.substringAfterLast('/'))
        }

        /**
         * Opens a `content://` URI from the Storage Access Framework.
         *
         * Streams from the provider's descriptor instead of copying the file into
         * the cache directory first — which for a 100 MB document would mean a
         * 100 MB copy and a doubled storage footprint before the first page draws.
         *
         * The descriptor is *detached*, and ownership moves to native code
         * unconditionally: it is closed when the document closes, and also closed
         * by the native side if the open fails. Deliberately **not** closed here
         * on the failure path — the fd number would by then be free for the OS to
         * reassign, and closing it again could take down an unrelated stream.
         */
        fun openUri(
            resolver: ContentResolver,
            uri: Uri,
            displayName: String,
            password: String? = null,
        ): PdfDocument {
            val pfd: ParcelFileDescriptor = resolver.openFileDescriptor(uri, "r")
                ?: throw PdfNativeException("could not open $uri for reading")

            // Everything between here and detachFd() must be infallible, or the
            // ParcelFileDescriptor leaks.
            val fd = try {
                pfd.detachFd()
            } catch (t: Throwable) {
                pfd.close()
                throw t
            }

            val handle = NativeBridge.openDocumentFd(fd, password)
            check(handle != NativeBridge.INVALID_HANDLE) { "native open returned no handle" }
            return PdfDocument(handle, displayName)
        }
    }
}

data class PageSize(val widthPoints: Float, val heightPoints: Float) {
    val aspectRatio: Float get() = if (heightPoints == 0f) 1f else widthPoints / heightPoints

    /** Pixel size at [zoom], matching the rounding the Rust core uses. */
    fun pixelSize(zoom: Float): Pair<Int, Int> =
        Math.round(widthPoints * zoom).coerceAtLeast(1) to
            Math.round(heightPoints * zoom).coerceAtLeast(1)
}
