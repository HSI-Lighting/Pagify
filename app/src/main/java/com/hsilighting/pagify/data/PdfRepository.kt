package com.hsilighting.pagify.data

import android.content.ContentResolver
import android.graphics.Bitmap
import android.net.Uri
import android.provider.OpenableColumns
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfDocument
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.core.RenderScale
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

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
    suspend fun renderPage(
        document: PdfDocument,
        pageIndex: Int,
        zoom: Float,
        rotationQuarterTurns: Int = 0,
    ): Bitmap = withContext(renderDispatcher) {
        val size = document.pageSize(pageIndex)
        val (width, height) = size.pixelSize(zoom).let { (w, h) ->
            if (rotationQuarterTurns % 2 == 0) w to h else h to w
        }

        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        document.renderPageInto(pageIndex, bitmap, zoom, rotationQuarterTurns)
        bitmap
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
