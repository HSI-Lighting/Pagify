package com.hsilighting.pagify.core

import android.content.ContentResolver
import android.graphics.Bitmap
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.compose.ui.geometry.Offset
import java.io.Closeable
import java.io.File
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

    /** Positioned text runs, for the highlighter. See [TextSegment]. */
    fun textSegments(pageIndex: Int): List<TextSegment> =
        TextSegment.listFromJson(NativeBridge.getTextSegmentsJson(requireOpen(), pageIndex))

    /**
     * The page's text with a box for every character, for selecting it.
     *
     * Heavier than [textSegments] and wanted far less often, which is why it is a
     * call of its own rather than part of the same trip.
     */
    fun pageCharacters(pageIndex: Int): PageCharacters =
        PageCharacters.fromJson(NativeBridge.getPageCharactersJson(requireOpen(), pageIndex))

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

    // --------------------------------------------------------------- capture --

    /**
     * Re-renders what was framed on screen and returns it encoded.
     *
     * **Not a screenshot** — the pixels come from the document, so nothing that is
     * not in the document can be in the result. See [NativeBridge.captureViewport]
     * and roadmap decision 4.8.
     *
     * Blocking, and slow enough to notice: a 4× capture of a dense page is a real
     * render. Call it off the main thread.
     */
    fun capture(request: CaptureRequest, markup: List<Markup> = emptyList()): ByteArray =
        NativeBridge.captureViewport(
            requireOpen(),
            request.tiles.tilesToWireJson(),
            request.width,
            request.height,
            request.scale.factor,
            request.background.toInt(),
            request.format.wireName,
            request.quality,
            markup.toWireJson(),
            request.mask.strokeToWireJson(),
        )

    /**
     * What the engine makes of a drawn stroke.
     *
     * Pure geometry, so unlike everything else here it takes no lock and can be
     * called straight off a gesture. Returns freehand when it is not sure, which
     * it is far more often than not — see [NativeBridge.recogniseStroke].
     */
    fun recogniseStroke(points: List<Offset>): MarkupShape =
        shapeFromWireJson(NativeBridge.recogniseStroke(points.strokeToWireJson()), points)

    // ------------------------------------------------------------------ edit --

    /** Applies one edit and returns the document's resulting [EditState]. */
    fun execute(command: PdfCommand): EditState =
        EditState.fromJson(NativeBridge.executeCommandJson(requireOpen(), command.toJson()))

    /** Reverses the most recent edit. A no-op when there is nothing to undo. */
    fun undo(): EditState = EditState.fromJson(NativeBridge.undoEdit(requireOpen()))

    fun redo(): EditState = EditState.fromJson(NativeBridge.redoEdit(requireOpen()))

    fun editState(): EditState = EditState.fromJson(NativeBridge.getEditStateJson(requireOpen()))

    /** Marks already in the file on this page, each with the engine's index. */
    fun savedMarks(pageIndex: Int, nextId: () -> Long): List<SavedMark> =
        savedMarksFromJson(
            NativeBridge.getAnnotationsJson(requireOpen(), pageIndex),
            pageIndex,
            nextId,
        )

    /** Persisted rotation of a page, in quarter turns. Zero if not editable. */
    fun pageRotation(pageIndex: Int): Int = NativeBridge.getPageRotation(requireOpen(), pageIndex)

    /**
     * Writes the edited document to [destination].
     *
     * [destination] must be a file this document was **not** opened from. PDFium
     * reads objects lazily for a document's entire life, so a save streams from the
     * source while writing to the sink; pointing both at one file truncates the
     * input halfway through and yields a PDF that is neither the old one nor the
     * new one. Overwriting the user's original is therefore a two-step job, which
     * is what [saveVia] does.
     *
     * @param incremental append a delta, leaving the original bytes intact so any
     *   existing digital signature stays valid. Passing false rewrites and compacts
     *   the file, which breaks every signature over it.
     */
    fun saveTo(destination: File, incremental: Boolean = true) {
        val pfd = ParcelFileDescriptor.open(
            destination,
            ParcelFileDescriptor.MODE_CREATE or
                ParcelFileDescriptor.MODE_TRUNCATE or
                ParcelFileDescriptor.MODE_WRITE_ONLY,
        )
        // detachFd, matching the open path: ownership moves to native code, which
        // closes it on every outcome. Closing it here too could take down a
        // descriptor the OS has since handed to something else.
        val fd = try {
            pfd.detachFd()
        } catch (t: Throwable) {
            pfd.close()
            throw t
        }
        NativeBridge.saveToFd(requireOpen(), fd, incremental)
    }

    /**
     * Saves to a scratch file, hands it to [publish], and deletes it afterwards.
     *
     * The scratch file is not a convenience — see [saveTo] for why writing over the
     * source directly destroys it. [publish] only runs once the save has finished,
     * so an interrupted save leaves the user's original untouched rather than
     * half-rewritten.
     *
     * The document keeps reading from its original descriptor afterwards, which
     * still refers to the pre-publish bytes. Callers must reopen to see the saved
     * file; the reader does that as part of its save action.
     */
    fun saveVia(scratchDir: File, incremental: Boolean = true, publish: (File) -> Unit) {
        val scratch = File.createTempFile("pagify-save", ".pdf", scratchDir)
        try {
            saveTo(scratch, incremental)
            publish(scratch)
        } finally {
            scratch.delete()
        }
    }

    // ----------------------------------------------------------------- cache --

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
