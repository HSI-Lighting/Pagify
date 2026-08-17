package com.hsilighting.pagify.ui.reader

import android.app.Application
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfDocument
import com.hsilighting.pagify.core.PdfPasswordException
import com.hsilighting.pagify.data.PdfRepository
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

class PdfReaderViewModel(application: Application) : AndroidViewModel(application) {

    private val repository = PdfRepository(application.contentResolver)

    private val _state = MutableStateFlow(PdfReaderState())
    val state: StateFlow<PdfReaderState> = _state.asStateFlow()

    private var document: PdfDocument? = null

    /** The URI is kept so a password retry can reopen without another file picker. */
    private var pendingUri: Uri? = null

    /** Cancelled and replaced whenever the page or zoom changes, so a fast scroll
     *  does not queue up prefetches for pages already scrolled past. */
    private var prefetchJob: Job? = null

    fun open(uri: Uri, password: String? = null) {
        pendingUri = uri
        closeDocument()
        _state.value = PdfReaderState(phase = PdfReaderState.Phase.Loading)

        viewModelScope.launch {
            try {
                val opened = repository.open(uri, password)
                document = opened

                val metadata = repository.metadata(opened)
                val firstPage = repository.pageSize(opened, 0)

                _state.update {
                    it.copy(
                        phase = PdfReaderState.Phase.Ready,
                        documentName = metadata.displayTitle(opened.sourceName),
                        metadata = metadata,
                        pageCount = opened.pageCount,
                        currentPage = 0,
                        pageSizes = mapOf(0 to firstPage),
                    )
                }
                schedulePrefetch()
            } catch (e: PdfPasswordException) {
                // Not a failure: the user has something to do about it.
                _state.update {
                    it.copy(phase = PdfReaderState.Phase.PasswordRequired(retry = e.isRetry))
                }
            } catch (t: Throwable) {
                Log.e(TAG, "could not open $uri", t)
                _state.update {
                    it.copy(
                        phase = PdfReaderState.Phase.Failed(
                            t.message ?: "This file could not be opened.",
                        ),
                    )
                }
            }
        }
    }

    fun submitPassword(password: String) {
        pendingUri?.let { open(it, password) }
    }

    /** Renders on demand for a page composable. Returns null if the page failed. */
    suspend fun renderPage(pageIndex: Int, zoom: Float): Bitmap? {
        val doc = document ?: return null
        return try {
            repository.renderPage(doc, pageIndex, zoom, state.value.rotationQuarterTurns)
        } catch (t: Throwable) {
            Log.w(TAG, "could not render page $pageIndex", t)
            null
        }
    }

    suspend fun pageSize(pageIndex: Int): PageSize? {
        state.value.pageSizes[pageIndex]?.let { return it }
        val doc = document ?: return null
        return try {
            repository.pageSize(doc, pageIndex).also { size ->
                _state.update { it.copy(pageSizes = it.pageSizes + (pageIndex to size)) }
            }
        } catch (t: Throwable) {
            Log.w(TAG, "could not measure page $pageIndex", t)
            null
        }
    }

    fun onPageVisible(pageIndex: Int) {
        if (pageIndex == _state.value.currentPage) return
        _state.update { it.copy(currentPage = pageIndex) }
        schedulePrefetch()
    }

    fun setZoom(zoom: Float) {
        val clamped = zoom.coerceIn(PdfReaderState.MIN_ZOOM, PdfReaderState.MAX_ZOOM)
        if (clamped == _state.value.zoom) return
        _state.update { it.copy(zoom = clamped) }
        schedulePrefetch()
    }

    /**
     * Multiply the current zoom by [factor] — what a pinch gesture reports.
     *
     * Relative rather than absolute on purpose. A gesture handler that computes
     * `currentZoom * factor` itself has to read the current zoom from somewhere,
     * and a `pointerInput` block captures its enclosing state exactly once; that
     * combination silently froze zoom at 1.0 in the first implementation. Reading
     * it here, from the single source of truth, removes the trap entirely.
     */
    fun zoomBy(factor: Float) {
        if (!factor.isFinite() || factor <= 0f) return
        setZoom(_state.value.zoom * factor)
    }

    /** Double-tap behaviour: jump to a readable zoom, or back to fit-width. */
    fun toggleZoom() {
        val current = _state.value.zoom
        setZoom(if (current > FIT_WIDTH_ZOOM + 0.01f) FIT_WIDTH_ZOOM else DOUBLE_TAP_ZOOM)
    }

    fun rotate() {
        _state.update { it.copy(rotationQuarterTurns = (it.rotationQuarterTurns + 1) % 4) }
        // Rotation changes every page's pixel dimensions, so nothing already
        // cached can be reused.
        document?.let { doc -> viewModelScope.launch { runCatching { doc.clearCache() } } }
        schedulePrefetch()
    }

    fun showMetadata(show: Boolean) = _state.update { it.copy(showMetadataSheet = show) }

    /**
     * Viewport width in device pixels, reported by the UI.
     *
     * Held as a plain field rather than in [PdfReaderState] because it changes only
     * on rotation or window resize and nothing renders differently *because* of it
     * — putting it in the state would trigger recompositions for no benefit.
     */
    private var viewportWidthPx: Float = 0f

    fun onViewportWidthChanged(widthPx: Float) {
        if (widthPx <= 0f || widthPx == viewportWidthPx) return
        viewportWidthPx = widthPx
        schedulePrefetch()
    }

    private fun schedulePrefetch() {
        val doc = document ?: return
        // Without a viewport width there is no way to know what scale to warm the
        // cache at, and prefetching at a guessed scale is worse than not
        // prefetching: it evicts useful entries to store ones nothing will ask for.
        val viewportWidth = viewportWidthPx
        if (viewportWidth <= 0f) return

        val snapshot = _state.value
        prefetchJob?.cancel()
        prefetchJob = viewModelScope.launch {
            val radius = PdfReaderState.PREFETCH_RADIUS
            val neighbours = (snapshot.currentPage - radius..snapshot.currentPage + radius)
                .filter { it != snapshot.currentPage }
            repository.prefetch(
                document = doc,
                pageIndices = neighbours,
                targetPixelWidth = viewportWidth * snapshot.zoom,
                rotationQuarterTurns = snapshot.rotationQuarterTurns,
            )
        }
    }

    private fun closeDocument() {
        prefetchJob?.cancel()
        document?.close()
        document = null
    }

    override fun onCleared() {
        super.onCleared()
        closeDocument()
    }

    private companion object {
        const val TAG = "PdfReaderViewModel"
        const val FIT_WIDTH_ZOOM = 1f
        const val DOUBLE_TAP_ZOOM = 2.5f
    }
}
