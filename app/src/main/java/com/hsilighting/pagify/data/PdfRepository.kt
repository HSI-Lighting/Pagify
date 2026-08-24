package com.hsilighting.pagify.data

import android.content.ContentResolver
import android.graphics.Bitmap
import android.net.Uri
import android.provider.OpenableColumns
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.EditState
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfCommand
import com.hsilighting.pagify.core.PdfDocument
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.core.NativeBridge
import com.hsilighting.pagify.core.PdfNativeException
import com.hsilighting.pagify.core.RenderScale
import com.hsilighting.pagify.core.SavedMark
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Boundary between the UI and the native engine.
 *
 * Its whole job is to keep native calls off the main thread. Rendering a page of
 * a dense document takes tens to hundreds of milliseconds; doing that on the main
 * thread is the difference between a smooth reader and a janky one.
 *
 * Note the dispatcher choice: [Dispatchers.Default] for rendering (CPU-bound,
 * bounded by core count) and [Dispatchers.IO] for opening (blocks on storage).
 */
class PdfRepository(
    private val resolver: ContentResolver,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
    private val renderDispatcher: CoroutineDispatcher = Dispatchers.Default,
) {

    /** Admits one thumbnail render at a time. See [renderThumbnail]. */
    private val thumbnailGate = Semaphore(1)

    suspend fun open(uri: Uri, password: String? = null): PdfDocument = withContext(ioDispatcher) {
        PdfDocument.openUri(resolver, uri, displayNameOf(uri), password)
    }

    suspend fun metadata(document: PdfDocument): PdfMetadata =
        withContext(ioDispatcher) { document.metadata }

    suspend fun pageSize(document: PdfDocument, pageIndex: Int): PageSize =
        withContext(ioDispatcher) { document.pageSize(pageIndex) }

    suspend fun pageText(document: PdfDocument, pageIndex: Int): String =
        withContext(renderDispatcher) { document.pageText(pageIndex) }

    /**
     * Renders a page to a freshly allocated bitmap.
     *
     * The bitmap is allocated here rather than reused from a pool: pages differ in
     * size, zoom changes their pixel dimensions, and Compose may still be drawing
     * the previous one when the next arrives. Recycling under those conditions is
     * how you get a `Canvas: trying to use a recycled bitmap` crash. The native
     * cache is what keeps this cheap — the allocation is the only copy involved.
     */
    /**
     * Renders a thumbnail, one at a time and skippable.
     *
     * The rail composes a dozen cells at once, and PDFium serialises internally
     * anyway — so letting twelve renders queue only piles up bitmaps and puts the
     * page you actually want behind all of them. This admits one at a time, and
     * `ensureActive` drops any request whose cell scrolled away while it waited,
     * which during a fast flick is nearly all of them.
     *
     * On a document with very heavy pages this is the difference between a rail
     * that scrolls and one that stalls: the cost of a thumbnail there is the page
     * load and image decode, not the handful of pixels drawn.
     */
    suspend fun renderThumbnail(
        document: PdfDocument,
        pageIndex: Int,
        zoom: Float,
    ): Bitmap = withContext(renderDispatcher) {
        thumbnailGate.withPermit {
            ensureActive()
            renderPageInternal(document, pageIndex, zoom, 0)
        }
    }

    suspend fun renderPage(
        document: PdfDocument,
        pageIndex: Int,
        zoom: Float,
        rotationQuarterTurns: Int = 0,
    ): Bitmap = withContext(renderDispatcher) {
        renderPageInternal(document, pageIndex, zoom, rotationQuarterTurns)
    }

    private fun renderPageInternal(
        document: PdfDocument,
        pageIndex: Int,
        zoom: Float,
        rotationQuarterTurns: Int,
    ): Bitmap {
        val size = document.pageSize(pageIndex)
        val (width, height) = size.pixelSize(zoom).let { (w, h) ->
            if (rotationQuarterTurns % 2 == 0) w to h else h to w
        }

        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        document.renderPageInto(pageIndex, bitmap, zoom, rotationQuarterTurns)
        return bitmap
    }

    /**
     * Warms the native cache for [pageIndices], sized for a page drawn
     * [targetPixelWidth] pixels wide.
     *
     * Takes a pixel width rather than a zoom factor because each page computes its
     * own render scale from its own dimensions — a document can mix portrait and
     * landscape pages, and one shared scale would be wrong for half of them.
     *
     * Failures are swallowed on purpose: a prefetch is an optimisation, and a page
     * that cannot be pre-rendered will simply report its error when the user
     * actually navigates to it.
     */
    suspend fun prefetch(
        document: PdfDocument,
        pageIndices: Iterable<Int>,
        targetPixelWidth: Float,
        rotationQuarterTurns: Int = 0,
    ) = withContext(renderDispatcher) {
        for (index in pageIndices) {
            if (index !in 0 until document.pageCount) continue
            runCatching {
                val scale = RenderScale.forPage(document.pageSize(index), targetPixelWidth)
                document.prefetchPage(index, scale, rotationQuarterTurns)
            }
        }
    }

    // ------------------------------------------------------------------ edit --

    /**
     * Applies one edit.
     *
     * On [ioDispatcher] rather than [renderDispatcher]: an edit takes the same
     * native registry lock a render takes, and deleting a page also copies its
     * content out so the deletion can be undone. Queuing that behind the render
     * pool would put it behind every page the user is currently looking at.
     */
    suspend fun execute(document: PdfDocument, command: PdfCommand): EditState =
        withContext(ioDispatcher) { document.execute(command) }

    suspend fun undo(document: PdfDocument): EditState =
        withContext(ioDispatcher) { document.undo() }

    suspend fun redo(document: PdfDocument): EditState =
        withContext(ioDispatcher) { document.redo() }

    suspend fun editState(document: PdfDocument): EditState =
        withContext(ioDispatcher) { document.editState() }

    suspend fun savedMarks(
        document: PdfDocument,
        pageIndex: Int,
        nextId: () -> Long,
    ): List<SavedMark> = withContext(ioDispatcher) { document.savedMarks(pageIndex, nextId) }

    suspend fun textMarks(document: PdfDocument, pageIndex: Int): List<Annotation.Text> =
        withContext(ioDispatcher) { document.textMarks(pageIndex) }

    suspend fun pageRotation(document: PdfDocument, pageIndex: Int): Int =
        withContext(ioDispatcher) { document.pageRotation(pageIndex) }

    /**
     * Writes the document to [uri] — the file it came from, or a new one.
     *
     * Two steps, and the order is the point: write a scratch file, and only once
     * that has completed copy it over the destination. PDFium reads lazily from the
     * source for the document's whole life, so saving straight back onto it would
     * truncate the input mid-save; and copying only after a complete write means an
     * interrupted save leaves the user's file untouched rather than half-rewritten.
     *
     * When [uri] is the document's own source, the open document still reads the
     * *old* descriptor afterwards, so the caller has to reopen to see what was
     * written.
     */
    suspend fun writeTo(
        document: PdfDocument,
        uri: Uri,
        scratchDir: File,
        incremental: Boolean = true,
    ) = withContext(ioDispatcher) {
        document.saveVia(scratchDir, incremental) { scratch ->
            // "rwt" truncates. Plain "w" leaves any bytes beyond the new length in
            // place, which on a shrinking save appends rubbish after %%EOF.
            resolver.openOutputStream(uri, "rwt")
                ?.use { out -> scratch.inputStream().use { it.copyTo(out) } }
                ?: throw PdfNativeException("could not open $uri for writing")
        }
    }

    /**
     * Write a new blank document to a destination the reader chose.
     *
     * Straight to the destination rather than through a scratch file: the save
     * path needs one because PDFium reads the source while writing, and here
     * there is no source to read.
     */
    suspend fun createBlank(
        uri: Uri,
        pages: Int,
        widthPoints: Float,
        heightPoints: Float,
        fill: Int,
        ruling: Int,
    ): Unit = withContext(ioDispatcher) {
        // "rwt" truncates, so choosing an existing file replaces it rather than
        // leaving the tail of the old one after the new %%EOF.
        resolver.openFileDescriptor(uri, "rwt")?.use { descriptor ->
            // Detached, because the native side adopts it. Closing the
            // ParcelFileDescriptor afterwards is then a no-op rather than a
            // double close.
            NativeBridge.createBlankDocument(
                descriptor.detachFd(),
                pages,
                widthPoints,
                heightPoints,
                fill,
                ruling,
            )
        } ?: throw PdfNativeException("could not open $uri for writing")
    }

    /**
     * The file's size in bytes, or 0 when the provider will not say.
     *
     * Zero rather than an error because plenty of providers legitimately do not
     * know — a stream, a generated document — and a size is a label on a list,
     * not something to fail an open over.
     */
    fun sizeOf(uri: Uri): Long {
        resolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            val column = cursor.getColumnIndex(OpenableColumns.SIZE)
            if (column >= 0 && cursor.moveToFirst() && !cursor.isNull(column)) {
                return cursor.getLong(column)
            }
        }
        // The provider would not say — which plenty legitimately do not for a
        // file they generated or streamed. The descriptor knows, and asking it
        // costs one open: worth it, because a row with no size looks like a
        // row with something missing.
        return runCatching {
            resolver.openFileDescriptor(uri, "r")?.use { it.statSize.coerceAtLeast(0L) } ?: 0L
        }.getOrDefault(0L)
    }

    /** The provider's display name, falling back to the URI's last path segment. */
    private fun displayNameOf(uri: Uri): String {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (column >= 0 && cursor.moveToFirst()) {
                cursor.getString(column)?.takeIf { it.isNotBlank() }?.let { return it }
            }
        }
        return uri.lastPathSegment?.substringAfterLast('/') ?: "Document"
    }
}
