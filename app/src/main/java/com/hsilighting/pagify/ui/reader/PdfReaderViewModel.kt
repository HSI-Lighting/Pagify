package com.hsilighting.pagify.ui.reader

import android.app.Application
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.util.Log
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationStore
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfDocument
import com.hsilighting.pagify.core.PdfPasswordException
import com.hsilighting.pagify.core.PenMode
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.core.RenderScale
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.ThumbnailCache
import com.hsilighting.pagify.data.PdfRepository
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.yield
import kotlin.math.abs

class PdfReaderViewModel(application: Application) : AndroidViewModel(application) {

    private val repository = PdfRepository(application.contentResolver)

    /** Survives scrolling, unlike a per-cell `remember`. Cleared on document change. */
    private val thumbnailCache = ThumbnailCache()

    /** Background job that fills [thumbnailCache] ahead of the reader. */
    private var thumbnailWarmJob: Job? = null

    /** Measures every page size up front; see measureAllPages. */
    private var pageMeasureJob: Job? = null

    /** Pages whose thumbnail render failed; not retried by the warmer. */
    private val unwarmablePages = mutableSetOf<Int>()

    /** On-demand thumbnail renders in flight. The warmer yields while this is nonzero. */
    private val interactiveRenders = AtomicInteger(0)

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
                warmThumbnails()
                measureAllPages(opened)
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

    /**
     * The most recent raster of each page, for handing straight to a view that is
     * about to be composed.
     *
     * Entering zoom builds a fresh view with no bitmap, and even the cheap proxy
     * takes a couple of hundred milliseconds — the blank-frame watcher measured
     * 54 ms of a completely empty content area in that gap. Reading a bitmap
     * synchronously at composition means the first frame already has pixels, so
     * there is no gap to measure.
     *
     * Only the last few pages are kept; this exists to bridge a view swap, not to
     * be another cache.
     */
    private val recentPageRasters = object : LinkedHashMap<Int, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Int, Bitmap>?): Boolean =
            size > RECENT_RASTER_COUNT
    }

    @Synchronized
    fun peekRenderedPage(pageIndex: Int): Bitmap? = recentPageRasters[pageIndex]

    @Synchronized
    private fun rememberRaster(pageIndex: Int, bitmap: Bitmap) {
        recentPageRasters[pageIndex] = bitmap
    }

    /** Renders on demand for a page composable. Returns null if the page failed. */
    suspend fun renderPage(pageIndex: Int, zoom: Float): Bitmap? {
        val doc = document ?: return null
        val startedAt = System.nanoTime()
        // Same signal the thumbnail warmer watches: the engine serialises all
        // PDFium calls internally, so a full-page render can still queue behind a
        // warmer thumbnail at the native level even though nothing on the Kotlin
        // side looks blocked. Counting this here lets the warmer yield to it too.
        interactiveRenders.incrementAndGet()
        return try {
            repository.renderPage(doc, pageIndex, zoom, state.value.rotationQuarterTurns)
                .also { bitmap ->
                    rememberRaster(pageIndex, bitmap)
                    SessionRecorder.record(
                        kind = "PAGE_PIXELS",
                        detail = "page=$pageIndex scale=$zoom " +
                            "px=${bitmap.width}x${bitmap.height} " +
                            "mp=${"%.1f".format(bitmap.width.toLong() * bitmap.height / 1e6)}",
                        durationMillis = (System.nanoTime() - startedAt) / 1_000_000,
                    )
                }
        } catch (t: CancellationException) {
            throw t
        } catch (t: Throwable) {
            Log.w(TAG, "could not render page $pageIndex", t)
            SessionRecorder.record("PAGE_FAIL", "page=$pageIndex ${t.message}")
            null
        } finally {
            interactiveRenders.decrementAndGet()
        }
    }

    // ------------------------------------------------------------- inspector --

    fun toggleRecording(externalFilesDir: File?): String? {
        if (SessionRecorder.isRecording) {
            val directory = externalFilesDir ?: return "No writable directory for the recording"
            val file = SessionRecorder.stop(directory)
            _state.update { it.copy(isRecording = false) }
            return file?.let { "Saved ${it.name}" }
        }

        SessionRecorder.start(
            documentName = _state.value.documentName.ifBlank { "(none)" },
            pageCount = _state.value.pageCount,
            deviceNote = "${Build.MODEL} | Android ${Build.VERSION.RELEASE} | " +
                "${Build.SUPPORTED_ABIS.firstOrNull()}",
        )
        _state.update { it.copy(isRecording = true) }
        return "Recording"
    }

    /**
     * Thumbnail for the rail, from memory when possible.
     *
     * The cache is checked before anything else touches the engine, so scrolling
     * back over pages already seen costs a map lookup rather than a page load —
     * which on a document with heavy pages is the difference between instant and
     * a couple of hundred milliseconds each.
     */
    suspend fun renderThumbnail(pageIndex: Int, zoom: Float): Bitmap? {
        thumbnailCache.get(pageIndex)?.let { cached ->
            SessionRecorder.record("THUMB_HIT", "page=$pageIndex", 0)
            return cached
        }

        val doc = document ?: return null
        SessionRecorder.record("THUMB_REQ", "page=$pageIndex scale=$zoom")
        val startedAt = System.nanoTime()
        interactiveRenders.incrementAndGet()

        return try {
            repository.renderThumbnail(doc, pageIndex, zoom).also { bitmap ->
                thumbnailCache.put(pageIndex, bitmap)
                SessionRecorder.record(
                    kind = "THUMB_RENDER",
                    detail = "page=$pageIndex px=${bitmap.width}x${bitmap.height} " +
                        "kb=${bitmap.byteCount / 1024}",
                    durationMillis = (System.nanoTime() - startedAt) / 1_000_000,
                )
            }
        } catch (t: CancellationException) {
            // Its cell scrolled away before this reached the front of the queue.
            SessionRecorder.record(
                kind = "THUMB_SKIP",
                detail = "page=$pageIndex",
                durationMillis = (System.nanoTime() - startedAt) / 1_000_000,
            )
            throw t
        } catch (t: Throwable) {
            Log.w(TAG, "could not render thumbnail $pageIndex", t)
            SessionRecorder.record("THUMB_FAIL", "page=$pageIndex ${t.message}")
            null
        } finally {
            interactiveRenders.decrementAndGet()
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
        val current = _state.value
        // Ignored while pinned: only the continuous list reports visibility, and a
        // stale report arriving as the view switches must not move the pinned page.
        if (current.zoomedPage != null) return
        if (pageIndex == current.currentPage) return
        _state.update { it.copy(currentPage = pageIndex) }
        schedulePrefetch()
    }

    /**
     * @param pinPage the page the gesture landed on. Used only when zoom first
     *   rises above fit-width; the page under the fingers is the one to pin, which
     *   is not necessarily the topmost visible one.
     */
    fun setZoom(zoom: Float, pinPage: Int? = null) {
        val clamped = zoom.coerceIn(PdfReaderState.MIN_ZOOM, PdfReaderState.MAX_ZOOM)
        if (clamped == _state.value.zoom) return

        _state.update { current ->
            // Crossing above fit-width pins the view to the page the gesture was
            // on; dropping back to fit-width releases it. Latching the page on the
            // way up (rather than tracking the visible page continuously) is what
            // keeps a magnified pan from drifting onto a neighbouring page.
            val zoomedPage = when {
                clamped <= PdfReaderState.FIT_WIDTH_ZOOM -> null
                current.zoomedPage != null -> current.zoomedPage
                else -> pinPage ?: current.currentPage
            }
            current.copy(
                zoom = clamped,
                zoomedPage = zoomedPage,
                // While pinned, the page being read is by definition the pinned one.
                currentPage = zoomedPage ?: current.currentPage,
            )
        }
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
    fun zoomBy(factor: Float, pinPage: Int? = null) {
        if (!factor.isFinite() || factor <= 0f) return
        setZoom(_state.value.zoom * factor, pinPage)
    }

    /**
     * Enter the pinned view on [pageIndex] at [targetZoom].
     *
     * The zoom is a parameter rather than a constant because a pinch and a
     * double-tap want different things. A double-tap is a request for a specific,
     * readable magnification. A pinch is continuous, and answering it with a fixed
     * jump throws the gesture away: however gently you pinched, the page snapped
     * to the double-tap zoom.
     */
    fun zoomInOn(pageIndex: Int, targetZoom: Float = DOUBLE_TAP_ZOOM) =
        setZoom(targetZoom, pinPage = pageIndex)

    /**
     * A settled zoom from the pinned view.
     *
     * Separate from [setZoom] only so it can be referenced as a plain
     * `(Float) -> Unit`; [setZoom]'s optional second parameter makes its method
     * reference the wrong shape.
     */
    fun zoomTo(zoom: Float) = setZoom(zoom)

    /** Double-tap behaviour: jump to a readable zoom, or back to fit-width. */
    fun toggleZoom() {
        val current = _state.value.zoom
        setZoom(
            if (current > PdfReaderState.FIT_WIDTH_ZOOM + 0.01f) {
                PdfReaderState.FIT_WIDTH_ZOOM
            } else {
                DOUBLE_TAP_ZOOM
            },
        )
    }

    fun rotate() {
        _state.update { it.copy(rotationQuarterTurns = (it.rotationQuarterTurns + 1) % 4) }
        // Rotation changes every page's pixel dimensions, so nothing already
        // cached can be reused.
        document?.let { doc -> viewModelScope.launch { runCatching { doc.clearCache() } } }
        schedulePrefetch()
    }

    fun showMetadata(show: Boolean) = _state.update { it.copy(showMetadataSheet = show) }

    fun toggleThumbnails() = _state.update { it.copy(showThumbnails = !it.showThumbnails) }

    // ---------------------------------------------------------- annotations --

    /**
     * The marks on the open document.
     *
     * Exposed directly rather than mirrored into [PdfReaderState] because a
     * stroke grows by a point every few milliseconds while drawing, and copying
     * the whole set into a new immutable state on each one would allocate
     * heavily mid-gesture. [PdfReaderState.annotationRevision] is what tells
     * Compose to redraw.
     */
    val annotations = AnnotationStore()

    /** Text runs per page, fetched once and reused; highlighting hit-tests these. */
    private val textSegmentCache = mutableMapOf<Int, List<TextSegment>>()

    fun selectTool(tool: AnnotationTool) {
        _state.update { it.copy(tool = tool) }
        // Recorded because tool state decides how every subsequent touch is
        // routed: with a tool live one finger annotates and the list stops
        // scrolling, so a recording without this cannot explain why a drag did
        // or did not scroll.
        SessionRecorder.record(
            kind = "TOOL_SELECT",
            detail = "tool=$tool penMode=${_state.value.penMode} " +
                "oneFingerPans=${tool == AnnotationTool.None}",
        )
    }

    fun setPenMode(mode: PenMode) = _state.update { current ->
        // Each mode carries its own palette, so a colour chosen for one would be
        // wrong for the other -- switching resets to that palette's default.
        val palette = when (mode) {
            PenMode.Highlight -> AnnotationColors.highlightPalette
            PenMode.Marker -> AnnotationColors.markerPalette
        }
        current.copy(
            penMode = mode,
            penColor = if (current.penColor in palette) current.penColor else palette.first(),
        )
    }

    fun setPenColor(color: Long) = _state.update { it.copy(penColor = color) }

    fun addAnnotation(annotation: Annotation) {
        val withId = when (annotation) {
            is Annotation.Highlight -> annotation.copy(id = annotations.nextId())
            is Annotation.Ink -> annotation.copy(id = annotations.nextId())
            is Annotation.Note -> annotation.copy(id = annotations.nextId())
            is Annotation.Signature -> annotation.copy(id = annotations.nextId())
        }
        annotations.add(withId)
        _state.update { it.copy(annotationRevision = it.annotationRevision + 1) }

        // The count is what distinguishes "the gesture never reached the tool"
        // from "it did, but produced nothing" — the two look identical on screen.
        val detail = when (withId) {
            is Annotation.Highlight -> "highlight page=${withId.pageIndex} lines=${withId.rects.size}"
            is Annotation.Ink -> "ink page=${withId.pageIndex} points=${withId.points.size}"
            is Annotation.Note -> "note page=${withId.pageIndex}"
            is Annotation.Signature -> "signature page=${withId.pageIndex}"
        }
        SessionRecorder.record("ANNOTATION_ADD", detail)
    }

    fun undoAnnotation() {
        if (annotations.undo()) {
            _state.update { it.copy(annotationRevision = it.annotationRevision + 1) }
        }
    }

    /**
     * Positioned text for a page, from cache when possible.
     *
     * Walking every run on a page is not free, and the highlighter asks for it on
     * the first touch of a drag — so it is fetched once per page and kept.
     */
    suspend fun textSegments(pageIndex: Int): List<TextSegment> {
        textSegmentCache[pageIndex]?.let { return it }
        val doc = document ?: return emptyList()
        return try {
            withContext(Dispatchers.Default) { doc.textSegments(pageIndex) }
                .also { textSegmentCache[pageIndex] = it }
        } catch (t: CancellationException) {
            throw t
        } catch (t: Throwable) {
            Log.w(TAG, "could not read text layout for page $pageIndex", t)
            emptyList()
        }
    }

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

    /**
     * Fills the thumbnail cache in the background, nearest-first.
     *
     * On a document with heavy pages a first thumbnail costs well over a second —
     * measured at a median of 1.5 s on the 2.9 GB catalogue — and no cache can
     * make a first render fast. What it can do is make sure the first render
     * already happened before you scrolled there.
     *
     * It always picks the uncached page closest to where you are, so jumping to
     * page 60 immediately redirects the work rather than grinding through pages
     * 4, 5, 6 that nobody is looking at. Each page acquires the render permit
     * separately, so an on-demand thumbnail or page slots in between rather than
     * queuing behind the whole document.
     */
    /**
     * Measures every page up front, so no page is ever laid out at a guess.
     *
     * Until this existed, a page that had not been measured was given a
     * placeholder A4-portrait aspect and then resized when its real dimensions
     * arrived. On a landscape document that is a ~2x error per page, and it
     * shifted the layout underneath `LazyColumn`'s anchor: jumping to a page
     * landed correctly and was then dragged away as the pages above it shrank.
     * A recording caught it as `firstVisible=62 offset=1188` becoming
     * `firstVisible=63 offset=233` within 250 ms.
     *
     * Affordable only because page sizing no longer loads the page — it reads the
     * page tree — so this is a few hundred cheap calls rather than a few hundred
     * page parses. Applied as one state update; 149 separate ones would each copy
     * the whole map and recompose the reader.
     */
    private fun measureAllPages(doc: PdfDocument) {
        pageMeasureJob?.cancel()
        pageMeasureJob = viewModelScope.launch {
            val measured = withContext(Dispatchers.Default) {
                buildMap {
                    for (index in 0 until doc.pageCount) {
                        if (!isActive) return@buildMap
                        runCatching { doc.pageSize(index) }.getOrNull()?.let { put(index, it) }
                    }
                }
            }
            if (measured.isEmpty()) return@launch
            // Existing entries win: anything the reader measured while this ran is
            // just as correct, and preferring them avoids a visible re-layout.
            _state.update { it.copy(pageSizes = measured + it.pageSizes) }
            SessionRecorder.record("PAGES_MEASURED", "count=${measured.size}")
        }
    }

    private fun warmThumbnails() {
        thumbnailWarmJob?.cancel()
        pageMeasureJob?.cancel()
        thumbnailWarmJob = viewModelScope.launch {
            val doc = document ?: return@launch
            val pageCount = doc.pageCount

            while (isActive) {
                // Yield to anything the user is waiting on.
                //
                // A recording showed why this is needed: with the warmer running,
                // on-demand thumbnails went from a 1.5 s median to 3.1 s, and some
                // requests sat 4 s before being cancelled. The warmer holds the
                // render permit for a whole page, so a thumbnail you are actually
                // looking at queued behind a page you are not — a priority
                // inversion that made the rail feel worse, not better.
                while (isActive && interactiveRenders.get() > 0) {
                    delay(WARM_YIELD_MILLIS)
                }

                val around = _state.value.currentPage
                val next = (0 until pageCount)
                    .filter { thumbnailCache.get(it) == null && it !in unwarmablePages }
                    .minByOrNull { abs(it - around) } ?: break

                val startedAt = System.nanoTime()
                val rendered = runCatching {
                    val size = repository.pageSize(doc, next)
                    repository.renderThumbnail(doc, next, RenderScale.thumbnailFor(size))
                }.getOrNull()

                if (rendered != null) {
                    thumbnailCache.put(next, rendered)
                    SessionRecorder.record(
                        kind = "THUMB_WARM",
                        detail = "page=$next cacheKb=${thumbnailCache.usedBytes / 1024}",
                        durationMillis = (System.nanoTime() - startedAt) / 1_000_000,
                    )
                } else {
                    // Remembered so the loop does not spin forever on a page that
                    // cannot be rendered. On-demand requests may still retry it.
                    unwarmablePages += next
                }
                yield()
            }
            SessionRecorder.record("THUMB_WARM_DONE", "cacheKb=${thumbnailCache.usedBytes / 1024}")
        }
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
        thumbnailWarmJob?.cancel()
        pageMeasureJob?.cancel()
        unwarmablePages.clear()
        document?.close()
        document = null
        // Thumbnails of a closed document are just held memory.
        thumbnailCache.clear()
        // Marks and text layout belong to the file that is going away; leaving
        // them would paint one document's annotations onto the next.
        annotations.clear()
        textSegmentCache.clear()
    }

    override fun onCleared() {
        super.onCleared()
        closeDocument()
    }

    private companion object {
        const val TAG = "PdfReaderViewModel"
        const val DOUBLE_TAP_ZOOM = 2.5f

        /** Enough to bridge a view swap on the current page and its neighbours. */
        const val RECENT_RASTER_COUNT = 4

        /** How often the warmer rechecks whether it may resume. */
        const val WARM_YIELD_MILLIS = 80L
    }
}
