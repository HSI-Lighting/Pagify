package com.hsilighting.pagify.ui.reader

import android.app.Application
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.util.Log
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.hsilighting.pagify.core.Annotation
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationEdit
import com.hsilighting.pagify.core.AnnotationStore
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.BitmapPools
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureRequest
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.CaptureTile
import com.hsilighting.pagify.core.captureFileName
import com.hsilighting.pagify.core.isWorthCapturing
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.AppSettings
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.defaultSize
import com.hsilighting.pagify.core.markupFor
import com.hsilighting.pagify.core.sizeRange
import com.hsilighting.pagify.core.EditState
import com.hsilighting.pagify.core.PageCharacters
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PageTextRecogniser
import com.hsilighting.pagify.core.NOTE_MARKER_RADIUS_POINTS
import com.hsilighting.pagify.core.PageRemap
import com.hsilighting.pagify.core.PdfCommand
import com.hsilighting.pagify.core.PdfDocument
import com.hsilighting.pagify.core.PdfPasswordException
import com.hsilighting.pagify.core.PenMode
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.core.ThemeChoice
import com.hsilighting.pagify.data.AppSettingsStore
import com.hsilighting.pagify.data.RecentDocumentsStore
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.core.RenderScale
import com.hsilighting.pagify.core.reorderForMove
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.ThumbnailCache
import com.hsilighting.pagify.data.PdfRepository
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
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

    /**
     * The library's list of documents opened before.
     *
     * Read once here rather than by the library screen, so that opening a document
     * from anywhere — a row, the picker, a share from another app — updates the
     * one list everything is drawn from.
     */
    private val recentDocuments = RecentDocumentsStore(application)
    val recents: StateFlow<List<RecentDocument>> = recentDocuments.documents

    /** Theme, viewfinder, and anything else that outlives a document. */
    private val settingsStore = AppSettingsStore(application)
    val settings: StateFlow<AppSettings> = settingsStore.settings

    fun setTheme(choice: ThemeChoice) {
        viewModelScope.launch { settingsStore.update { it.copy(theme = choice) } }
    }

    /** The hard off: no viewfinder while zoomed, and no handle to bring one back. */
    fun setShowViewfinder(show: Boolean) {
        viewModelScope.launch { settingsStore.update { it.copy(showViewfinder = show) } }
    }

    /** Where the folded handle was dragged to, in fractions of the reader. */
    fun setViewfinderHandle(position: Offset) {
        viewModelScope.launch {
            settingsStore.update {
                it.copy(
                    viewfinderHandleX = position.x.coerceIn(0f, 1f),
                    viewfinderHandleY = position.y.coerceIn(0f, 1f),
                )
            }
        }
    }

    /** The soft one: collapse it to its handle, or open it again. */
    fun setViewfinderMinimized(minimized: Boolean) {
        viewModelScope.launch { settingsStore.update { it.copy(viewfinderMinimized = minimized) } }
    }

    init {
        viewModelScope.launch { recentDocuments.load() }
        viewModelScope.launch { settingsStore.load() }
    }

    /**
     * Close the document and go back to the library.
     *
     * The reader keeps nothing once the document is gone — see [closeDocument] —
     * so this is a full reset rather than a screen change with state left behind.
     */
    fun returnToLibrary() {
        closeDocument()
        pendingUri = null
        _state.value = PdfReaderState(
            documentRevision = _state.value.documentRevision,
            showThumbnails = _state.value.showThumbnails,
        )
    }

    /** Forget every document in the library. */
    fun clearLibrary() {
        viewModelScope.launch { recentDocuments.clear() }
    }

    /** Drop a document from the library — moved, deleted, or no longer permitted. */
    fun forgetDocument(uri: String) {
        viewModelScope.launch { recentDocuments.forget(uri) }
    }

    fun open(uri: Uri, password: String? = null) {
        pendingUri = uri
        closeDocument()
        _state.value = PdfReaderState(
            phase = PdfReaderState.Phase.Loading,
            // Carried across the reset so it keeps increasing; a fresh state would
            // put it back to zero and the effects keyed on it would not re-run.
            documentRevision = _state.value.documentRevision,
            // Carried for the same reason: the rail is hidden once on a
            // narrow screen, and a fresh state would put it back over the
            // page every time a document was opened. The hide is a
            // decision about the screen, not about the document.
            showThumbnails = _state.value.showThumbnails,
        )

        viewModelScope.launch {
            try {
                val opened = repository.open(uri, password)
                document = opened

                val metadata = repository.metadata(opened)
                val firstPage = repository.pageSize(opened, 0)
                // Fetched on open so the UI knows straight away whether this
                // document can be edited at all, rather than offering the controls
                // and discovering it cannot when one is pressed.
                val edits = repository.editState(opened)

                _state.update {
                    it.copy(
                        phase = PdfReaderState.Phase.Ready,
                        documentName = metadata.displayTitle(opened.sourceName),
                        metadata = metadata,
                        pageCount = opened.pageCount,
                        currentPage = 0,
                        pageSizes = mapOf(0 to firstPage),
                        editState = edits,
                        documentRevision = it.documentRevision + 1,
                    )
                }
                // Remembered only once it has actually opened. A document that
                // failed, or that asked for a password and never got one, is not
                // something to offer again from the library as if it worked.
                recentDocuments.remember(
                    RecentDocument(
                        uri = uri.toString(),
                        name = opened.sourceName,
                        sizeBytes = repository.sizeOf(uri),
                        pageCount = opened.pageCount,
                        openedAtMillis = System.currentTimeMillis(),
                    ),
                )

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
        // Bounded by *bytes* as well as by count. Four entries sounds modest
        // until the entries are full-page rasters: a page measured 4465 x 3157 on
        // the test tablet is ~54 MB at ARGB_8888, so a count-only cap permitted
        // roughly 215 MB — more than the engine's 160 MB cache and the 48 MB
        // thumbnail cache combined, in the one pool nothing was watching.
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Int, Bitmap>?): Boolean =
            size > RECENT_RASTER_COUNT ||
                values.sumOf { it.byteCount } > RECENT_RASTER_BUDGET_BYTES
    }

    /**
     * Registers the raster map with the process-wide accounting.
     *
     * It exists only to bridge a view swap — handing a bitmap to a composable
     * about to be built — so under pressure it is dropped outright rather than
     * trimmed by half. The cost of losing it is one cheap proxy render, and the
     * blank-frame watcher will say if that is ever visible.
     */
    private val rasterPool = object : BitmapPools.Pool {
        override val poolName = "recentRasters"

        override fun bytesHeld(): Int = heldRasterBytes()

        override fun trimTo(level: Int) = dropRecentRasters()
    }.also { BitmapPools.register(it) }

    @Synchronized
    private fun heldRasterBytes(): Int = recentPageRasters.values.sumOf { it.byteCount }

    @Synchronized
    private fun dropRecentRasters() = recentPageRasters.clear()

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

    /** Set the rail directly, which is what a settings switch means. */
    fun setThumbnails(show: Boolean) = _state.update { it.copy(showThumbnails = show) }

    /**
     * The reader is on a narrow screen; put the thumbnail rail away.
     *
     * Once, and only once. The rail is 104dp, which is a quarter of a phone in
     * portrait and takes that quarter from the page — but a reader who turns it
     * back on means it, and a rule that hid it on every recomposition would be
     * arguing with them.
     */
    fun onNarrowScreen() {
        if (railHiddenForNarrowScreen) return
        railHiddenForNarrowScreen = true
        _state.update { it.copy(showThumbnails = false) }
    }

    private var railHiddenForNarrowScreen = false

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

    /** Reads text off pages that carry none. See [PageTextRecogniser]. */
    private val recogniser = PageTextRecogniser()

    /** Recognition in flight, keyed by page, so a page is never read twice at once. */
    private val recognitionJobs = mutableMapOf<Int, Deferred<List<TextSegment>>>()

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

    /** The Note tool was tapped at [anchor] on [pageIndex]; ask for the text. */
    fun requestNote(pageIndex: Int, anchor: Offset) = _state.update {
        it.copy(pendingNote = PendingNote(pageIndex, anchor))
    }

    fun cancelNote() = _state.update { it.copy(pendingNote = null) }

    fun openNote(note: Annotation.Note) = _state.update { it.copy(openNote = note) }

    fun closeNote() = _state.update { it.copy(openNote = null) }

    /** Rub out the note being read, from the store and from the file if it is in it. */
    fun deleteOpenNote() {
        val note = _state.value.openNote ?: return
        _state.update { it.copy(openNote = null) }
        annotations.eraseAt(note.pageIndex, note.anchor, NOTE_MARKER_RADIUS_POINTS)
        refreshAnnotations()
        commitErasedSavedMarks(note.pageIndex)
    }

    /**
     * Place the note that was being typed.
     *
     * Blank text places nothing. An empty note draws as a marker with no content,
     * which is indistinguishable from a stray tap and impossible to tell from a
     * bug once it is sitting on the page.
     */
    fun confirmNote(text: String) {
        val pending = _state.value.pendingNote ?: return
        _state.update { it.copy(pendingNote = null) }
        if (text.isBlank()) return

        addAnnotation(
            Annotation.Note(
                id = 0L,
                pageIndex = pending.pageIndex,
                anchor = pending.anchor,
                text = text.trim(),
                color = _state.value.penColor,
            ),
        )
    }

    fun addAnnotation(annotation: Annotation) {
        val withId = when (annotation) {
            is Annotation.Highlight -> annotation.copy(id = annotations.nextId())
            is Annotation.Ink -> annotation.copy(id = annotations.nextId())
            is Annotation.Note -> annotation.copy(id = annotations.nextId())
            is Annotation.Signature -> annotation.copy(id = annotations.nextId())
        }
        annotations.add(withId)
        refreshAnnotations()

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

    // ------------------------------------------------------------- the eraser --

    /** Open an eraser stroke; everything it rubs out until [endErase] is one undo. */
    fun beginErase() = annotations.beginErase()

    /**
     * @param tolerance in page points. The caller converts a fixed touch radius
     *   through the current render scale, so the target stays the same physical
     *   size however far the page is magnified.
     */
    fun eraseAt(pageIndex: Int, point: Offset, tolerance: Float) {
        val hit = annotations.eraseAt(pageIndex, point, tolerance)
        Log.i(
            TAG,
            "erase: page=$pageIndex at=${point.x},${point.y} tol=$tolerance hit=$hit " +
                "marks=${annotations.forPage(pageIndex).map { m ->
                    when (m) {
                        is Annotation.Highlight -> "H${m.rects.firstOrNull()}"
                        is Annotation.Ink -> "I${m.points.size}"
                        is Annotation.Note -> "N${m.anchor}"
                        is Annotation.Signature -> "S"
                    }
                }}",
        )
        if (hit) {
            refreshAnnotations()
            // A mark that came out of the file has to come out of the file.
            commitErasedSavedMarks(pageIndex)
        }
        // Misses are recorded too, and matter more than hits: "the gesture never
        // reached the tool" and "it arrived and found nothing there" are the same
        // picture on screen, and only the second one is about the hit test.
        SessionRecorder.record(
            kind = "ERASE",
            detail = "page=$pageIndex at=${point.x.toInt()},${point.y.toInt()} " +
                "tol=${tolerance.toInt()} hit=$hit left=${annotations.countOnPage(pageIndex)}",
        )
    }

    fun endErase() {
        annotations.endErase()
        refreshAnnotations()
    }

    fun clearPage(pageIndex: Int) {
        val cleared = annotations.clearPage(pageIndex)
        if (cleared > 0) {
            refreshAnnotations()
            SessionRecorder.record("ANNOTATION_CLEAR", "page=$pageIndex marks=$cleared")
        }
    }

    fun clearAllAnnotations() {
        val cleared = annotations.clearAll()
        if (cleared > 0) {
            refreshAnnotations()
            SessionRecorder.record("ANNOTATION_CLEAR", "scope=document marks=$cleared")
        }
    }

    // --------------------------------------------------------- undo and redo --

    fun undoAnnotation() = applyHistory(annotations.undo(), "UNDO")

    fun redoAnnotation() = applyHistory(annotations.redo(), "REDO")

    /**
     * Reflect a history step, and take the reader to the page it changed.
     *
     * Undoing something on a page you are not looking at is otherwise silent —
     * the change happens where you cannot see it, which reads as a broken button
     * rather than as a change off screen.
     */
    private fun applyHistory(edit: AnnotationEdit?, kind: String) {
        if (edit == null) return
        refreshAnnotations()
        SessionRecorder.record(
            kind = "ANNOTATION_$kind",
            detail = "what=${edit.label} marks=${edit.size} page=${edit.pageIndex}",
        )
        edit.pageIndex?.let { page ->
            if (page != _state.value.currentPage) {
                _state.update { it.copy(currentPage = page, jumpToPage = page) }
            }
        }
    }

    /** The reader has taken the scroll a history step asked for. */
    fun jumpHandled() = _state.update { it.copy(jumpToPage = null) }

    // -------------------------------------------------------------- selection --

    /**
     * Per-page character geometry, fetched on demand.
     *
     * Not fetched with the page: a dense page is thousands of boxes, and most
     * pages are read rather than selected from. Cached once asked for, because a
     * handle drag asks again on every move.
     */
    private val characterCache = mutableMapOf<Int, PageCharacters>()

    private suspend fun charactersOf(pageIndex: Int): PageCharacters {
        characterCache[pageIndex]?.let { return it }
        val doc = document ?: return PageCharacters.EMPTY

        val characters = withContext(Dispatchers.Default) {
            runCatching { doc.pageCharacters(pageIndex) }.getOrElse { failure ->
                Log.e(TAG, "reading characters on page $pageIndex failed", failure)
                PageCharacters.EMPTY
            }
        }
        characterCache[pageIndex] = characters
        return characters
    }

    /**
     * Select the word under a long press.
     *
     * A word rather than a character: pointing at a letter means pointing at the
     * word it is in, and starting from one character would mean dragging a handle
     * before anything useful was selected.
     */
    fun selectWordAt(pageIndex: Int, point: Offset) {
        viewModelScope.launch {
            val characters = charactersOf(pageIndex)
            val index = characters.indexNear(point)
            if (index == null) {
                // A page with no text layer. The highlighter has a hint for this,
                // but it only shows while the highlighter is armed — and a long
                // press needs no tool, so without this the gesture would simply
                // do nothing and look broken. Which is what it looked like on a
                // catalogue where 92 of 95 pages are outlined type.
                noteHasNoSelectableText(pageIndex)
                SessionRecorder.record("SELECT_NO_TEXT", "page=$pageIndex")
                _state.update { it.copy(message = "No selectable text here — the page is an image or outlines.") }
                return@launch
            }
            applySelection(pageIndex, characters, characters.wordAround(index))
        }
    }

    /**
     * Drag one end of the selection.
     *
     * The two ends are interchangeable: dragging the start past the end turns the
     * selection round, which is what happens if you keep going.
     */
    fun moveSelectionHandle(isStart: Boolean, point: Offset) {
        val current = _state.value.selection ?: return
        viewModelScope.launch {
            val characters = charactersOf(current.pageIndex)
            val moved = characters.indexNear(point) ?: return@launch
            val anchor = if (isStart) current.range.last else current.range.first
            applySelection(
                current.pageIndex,
                characters,
                minOf(anchor, moved)..maxOf(anchor, moved),
            )
        }
    }

    private fun applySelection(pageIndex: Int, characters: PageCharacters, range: IntRange) {
        _state.update {
            it.copy(
                selection = PageTextSelection(
                    pageIndex = pageIndex,
                    range = range,
                    rects = characters.rectsOf(range),
                    text = characters.textOf(range),
                ),
            )
        }
    }

    fun clearSelection() = _state.update { it.copy(selection = null) }

    /** Put the selected text on the clipboard. */
    fun copySelection() {
        val selection = _state.value.selection ?: return
        if (selection.text.isBlank()) return

        val clipboard = getApplication<Application>()
            .getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("Pagify", selection.text))

        SessionRecorder.record(
            kind = "TEXT_COPY",
            detail = "page=${selection.pageIndex} chars=${selection.text.length}",
        )
        _state.update {
            it.copy(
                selection = null,
                // Android 13 and later shows its own copy confirmation, so saying
                // it again would be the second one. Older versions say nothing.
                message = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    it.message
                } else {
                    "Copied."
                },
            )
        }
    }

    /** Turn the selection into a highlight, in the current pen colour. */
    fun highlightSelection() {
        val selection = _state.value.selection ?: return
        if (selection.rects.isEmpty()) return

        addAnnotation(
            Annotation.Highlight(
                id = 0L,
                pageIndex = selection.pageIndex,
                rects = selection.rects,
                color = _state.value.penColor,
            ),
        )
        clearSelection()
    }

    /**
     * Republish everything derived from the store.
     *
     * The store is mutable and identity-stable, so nothing in it is observable
     * until the revision counter moves. `canUndo` and `canRedo` are copied out at
     * the same moment, or the buttons would enable a frame late.
     */
    private fun refreshAnnotations() = _state.update {
        it.copy(
            annotationRevision = it.annotationRevision + 1,
            canUndo = annotations.canUndo,
            canRedo = annotations.canRedo,
            annotationsOnPage = annotations.countOnPage(it.currentPage),
            annotationsInDocument = annotations.total,
            // Marks read out of the file are already in it. Counting them as
            // unsaved made a document with saved marks claim unsaved changes the
            // moment it opened.
            unsavedMarkCount = (annotations.total - savedMarkLocations.size).coerceAtLeast(0),
        )
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
        val startedAt = System.nanoTime()

        val embedded = try {
            withContext(Dispatchers.Default) { doc.textSegments(pageIndex) }
        } catch (t: CancellationException) {
            throw t
        } catch (t: Throwable) {
            Log.w(TAG, "could not read text layout for page $pageIndex", t)
            emptyList()
        }

        SessionRecorder.record(
            kind = "TEXT_LAYER",
            detail = "page=$pageIndex runs=${embedded.size}",
            durationMillis = (System.nanoTime() - startedAt) / 1_000_000,
        )

        if (embedded.isNotEmpty()) {
            textSegmentCache[pageIndex] = embedded
            return embedded
        }

        // No text layer at all. Read the page instead — see PageTextRecogniser for
        // how a document can look entirely like text and contain none.
        return recognisedSegments(doc, pageIndex)
    }

    /**
     * Recognised text for a page, run at most once per page.
     *
     * Shared through a `Deferred` rather than merely guarded, because the
     * highlighter asks for a page's text on the first touch of a drag and can ask
     * again before the first answer arrives. Without this, a page could be rendered
     * at recognition scale and read twice over — the most expensive thing this app
     * does, duplicated at exactly the moment the user is waiting on it.
     */
    private suspend fun recognisedSegments(
        doc: PdfDocument,
        pageIndex: Int,
    ): List<TextSegment> {
        val job = synchronized(recognitionJobs) {
            recognitionJobs.getOrPut(pageIndex) {
                viewModelScope.async {
                    try {
                        runRecognition(doc, pageIndex)
                    } finally {
                        synchronized(recognitionJobs) { recognitionJobs.remove(pageIndex) }
                    }
                }
            }
        }
        return job.await()
    }

    private suspend fun runRecognition(doc: PdfDocument, pageIndex: Int): List<TextSegment> {
        _state.update { it.copy(pagesBeingRecognised = it.pagesBeingRecognised + pageIndex) }
        val startedAt = System.nanoTime()

        return try {
            val size = repository.pageSize(doc, pageIndex)
            // Rendered specifically for recognition rather than reusing whatever
            // raster is on screen: accuracy depends on glyph height in pixels, and
            // the displayed page may be at any zoom, including a proxy pass.
            val bitmap = repository.renderPage(doc, pageIndex, PageTextRecogniser.scaleFor(size))
            val segments = recogniser.recognise(bitmap, size)

            textSegmentCache[pageIndex] = segments
            SessionRecorder.record(
                kind = "TEXT_RECOGNISED",
                detail = "page=$pageIndex runs=${segments.size} px=${bitmap.width}x${bitmap.height}",
                durationMillis = (System.nanoTime() - startedAt) / 1_000_000,
            )

            // Recorded so the reader can say the text was read off the page rather
            // than found in it. Recognition makes mistakes, and a user who selects
            // a wrong character deserves to know why.
            if (segments.isNotEmpty()) {
                _state.update { it.copy(pagesRecognised = it.pagesRecognised + pageIndex) }
            } else {
                noteHasNoSelectableText(pageIndex)
            }
            segments
        } catch (t: CancellationException) {
            throw t
        } catch (t: Throwable) {
            Log.w(TAG, "could not recognise text on page $pageIndex", t)
            // Deliberately not cached: a failure here is usually transient — memory
            // pressure, or the model still unpacking on first use — and caching it
            // would leave the page unselectable for the rest of the session.
            noteHasNoSelectableText(pageIndex)
            emptyList()
        } finally {
            _state.update { it.copy(pagesBeingRecognised = it.pagesBeingRecognised - pageIndex) }
        }
    }

    /**
     * Remember that a page carries no text layer.
     *
     * Some pages have nothing to select even after recognition has been tried:
     * artwork with no words in it, or a photograph. The highlighter then produces
     * nothing, and the UI needs to be able to say so rather than let the tool look
     * broken.
     *
     * Note this is now the *second* answer, not the first. A page with no text
     * layer is read by [PageTextRecogniser] before it is called textless — which
     * is what the 2.97 GB catalogue needed, where the words on 92 of 95 pages are
     * vector outlines rather than text.
     */
    fun noteHighlightFoundNothing(pageIndex: Int) = noteHasNoSelectableText(pageIndex)

    private fun noteHasNoSelectableText(pageIndex: Int) {
        // A page that does have text, recognised or embedded, must never be marked
        // as having none — and the highlighter reports a miss on any drag that hits
        // nothing, including one across the blank half of a page. Without this,
        // recognising a page and then dragging beside a word would put up "no
        // selectable text" on the very page that had just been read successfully.
        if (textSegmentCache[pageIndex]?.isNotEmpty() == true) return

        _state.update {
            if (pageIndex in it.pagesWithoutSelectableText) it
            else it.copy(pagesWithoutSelectableText = it.pagesWithoutSelectableText + pageIndex)
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
        // Keyed by page index, so keeping it would let one document's characters
        // answer for another's page 3.
        characterCache.clear()
        // Marks belong to the file that is going away, and so does the mapping
        // from their ids to indices in it.
        savedMarkLocations.clear()
        loadedMarkPages.clear()
    }

    override fun onCleared() {
        super.onCleared()
        closeDocument()
    }


    // -------------------------------------------------------- document edits --

    fun showPageOrganiser(show: Boolean) = _state.update { it.copy(showPageOrganiser = show) }

    fun messageShown() = _state.update { it.copy(message = null) }

    fun deletePage(pageIndex: Int) {
        val doc = document ?: return
        if (_state.value.pageCount <= 1) {
            _state.update { it.copy(message = "A document must keep at least one page.") }
            return
        }
        runEdit(
            command = PdfCommand.DeletePage(pageIndex),
            remap = PageRemap.afterDelete(pageIndex),
            describe = { "Deleted page ${pageIndex + 1}" },
            document = doc,
        )
    }

    fun insertBlankPage(at: Int) {
        val doc = document ?: return
        // A new page matches the one it is inserted before, so it does not appear
        // as an odd-sized sheet in the middle of a uniform document. Falling back
        // to A4 only when nothing has been measured yet.
        val template = _state.value.pageSizes[at.coerceAtMost(_state.value.pageCount - 1)]
            ?: _state.value.pageSizes.values.firstOrNull()
            ?: PageSize(A4_WIDTH_POINTS, A4_HEIGHT_POINTS)

        runEdit(
            command = PdfCommand.InsertBlankPage(at, template.widthPoints, template.heightPoints),
            remap = PageRemap.afterInsert(at),
            describe = { "Inserted page ${at + 1}" },
            document = doc,
        )
    }

    /**
     * Move one page to a new position.
     *
     * The engine takes a whole permutation rather than a from/to pair, so the drag
     * is converted here: remove the moved index and re-insert it, then read off
     * where each original index ended up. Doing it this way means the reorder that
     * reaches the document is the same shape whether it came from a drag, a
     * keyboard shortcut or a script.
     */
    fun movePage(from: Int, to: Int) {
        val doc = document ?: return
        val count = _state.value.pageCount
        if (from == to || from !in 0 until count || to !in 0 until count) return

        val order = reorderForMove(count, from, to)

        runEdit(
            command = PdfCommand.ReorderPages(order),
            remap = PageRemap.afterReorder(order),
            describe = { "Moved page ${from + 1} to ${to + 1}" },
            document = doc,
            follow = to,
        )
    }

    /**
     * Turn a page and write the turn into the document.
     *
     * Distinct from [rotateView], which turns the whole document on screen and
     * changes nothing on disk.
     */
    fun rotatePage(pageIndex: Int, quarterTurns: Int = 1) {
        val doc = document ?: return
        val size = _state.value.pageSizes[pageIndex]

        viewModelScope.launch {
            try {
                val current = repository.pageRotation(doc, pageIndex)
                val turned = ((current + quarterTurns) % 4 + 4) % 4
                val state = repository.execute(
                    doc,
                    PdfCommand.SetPageRotation(pageIndex, turned),
                )
                // Marks are stored in page points, so they have to turn with the
                // page or they land somewhere else entirely on it.
                if (size != null) {
                    annotations.rotatePage(
                        pageIndex,
                        quarterTurns,
                        size.widthPoints,
                        size.heightPoints,
                    )
                }
                afterEdit(doc, state, "Rotated page ${pageIndex + 1}", follow = pageIndex)
            } catch (t: Throwable) {
                reportEditFailure(t)
            }
        }
    }

    fun undoEdit() {
        val doc = document ?: return
        viewModelScope.launch {
            try {
                val label = _state.value.editState.undoLabel
                val state = repository.undo(doc)
                // The page tree moved in a way this layer cannot describe, so every
                // index-keyed mark is suspect. Dropping the annotation history is
                // the honest response; see AnnotationStore.remapPages.
                annotations.clearAll()
                afterEdit(doc, state, label?.let { "Undid: $it" })
            } catch (t: Throwable) {
                reportEditFailure(t)
            }
        }
    }

    fun redoEdit() {
        val doc = document ?: return
        viewModelScope.launch {
            try {
                val label = _state.value.editState.redoLabel
                val state = repository.redo(doc)
                annotations.clearAll()
                afterEdit(doc, state, label?.let { "Redid: $it" })
            } catch (t: Throwable) {
                reportEditFailure(t)
            }
        }
    }

    /**
     * Write the edits back over the file the user opened.
     *
     * Reopens afterwards rather than carrying on with the open document. The
     * document reads lazily from the descriptor it was opened with, which still
     * points at the *pre-save* bytes; continuing to use it would show the old file
     * while the new one sits on disk, and the next edit would be built on a stale
     * view of it.
     *
     * @param incremental append a delta, keeping the original bytes and any
     *   signature over them intact. False rewrites and compacts the file.
     */
    fun save(incremental: Boolean = true) {
        val doc = document ?: return
        val uri = pendingUri ?: return
        if (_state.value.isSaving) return
        if (!_state.value.editState.dirty && _state.value.unsavedMarkCount == 0) {
            _state.update { it.copy(message = "No changes to save.") }
            return
        }

        _state.update { it.copy(isSaving = true) }
        viewModelScope.launch {
            try {
                commitMarks(doc)
                repository.writeTo(doc, uri, scratchDir(), incremental)
                SessionRecorder.record("SAVED", "incremental=$incremental")
                // Reopen at the page the user was on, so saving does not feel like
                // closing and re-opening the file.
                val page = _state.value.currentPage
                _state.update { it.copy(isSaving = false, message = "Saved.") }
                open(uri)
                _state.update { it.copy(jumpToPage = page) }
            } catch (t: Throwable) {
                Log.e(TAG, "save failed", t)
                _state.update { it.copy(isSaving = false, message = saveFailureMessage(t)) }
            }
        }
    }

    /**
     * What to tell the user when a save fails.
     *
     * A `SecurityException` here is not an error on the user's part and not
     * something they can fix by trying again: a PDF opened from a mail attachment
     * or a download arrives with a read-only grant, and nothing the app does later
     * can widen it. So it names the way out instead of repeating the platform's
     * message, which on a device read
     * "com.hsilighting.pagify has no access to content://media/external/file/1000001372".
     */
    private fun saveFailureMessage(t: Throwable): String = when (t) {
        is SecurityException -> "This file is read-only. Use Save a copy instead."
        else -> t.message ?: "The document could not be saved."
    }

    /**
     * Write the edits to a file the user has just chosen.
     *
     * The way out when the original cannot be written to: a PDF arriving from a
     * mail attachment or a browser download is handed over read-only, and no
     * amount of permission-taking on our side changes that. Saving a copy is then
     * the only honest option, so it is offered rather than reporting a failure the
     * user can do nothing about.
     *
     * Deliberately does not reopen: the user keeps editing the document they had
     * open, and the copy is a snapshot of it.
     */
    fun saveCopyTo(destination: Uri) {
        val doc = document ?: return
        if (_state.value.isSaving) return

        _state.update { it.copy(isSaving = true) }
        viewModelScope.launch {
            try {
                repository.writeTo(doc, destination, scratchDir(), incremental = true)
                SessionRecorder.record("SAVED_COPY", "to=$destination")
                _state.update { it.copy(isSaving = false, message = "Copy saved.") }
            } catch (t: Throwable) {
                Log.e(TAG, "save copy failed", t)
                _state.update {
                    it.copy(
                        isSaving = false,
                        message = t.message ?: "The copy could not be saved.",
                    )
                }
            }
        }
    }

    /**
     * Write the session's marks into the document, just before it is saved.
     *
     * The commit boundary is the save, not the gesture. Drawing a stroke sends
     * nothing across JNI — a marker line is hundreds of points and would take the
     * engine's lock hundreds of times mid-drag — so marks accumulate in the
     * [AnnotationStore] and cross once, here, as commands.
     *
     * Each one goes through the ordinary command path, so a failure part-way
     * leaves the marks that did land already recorded in the document's own undo
     * history rather than in some half-state of this function's making.
     */
    private suspend fun commitMarks(doc: PdfDocument) {
        for (page in annotations.pagesWithMarks()) {
            for (mark in annotations.forPage(page)) {
                // Marks read out of the document are already in it. Writing them
                // again on every save would duplicate every highlight each time
                // the reader pressed Save.
                if (savedMarkLocations.containsKey(mark.id)) continue
                repository.execute(doc, PdfCommand.AddAnnotation(page, mark))
            }
        }
    }

    /** Where [PdfDocument.saveVia] puts its scratch copy. */
    private fun scratchDir(): File = getApplication<Application>().cacheDir

    private fun runEdit(
        command: PdfCommand,
        remap: PageRemap,
        describe: () -> String,
        document: PdfDocument,
        follow: Int? = null,
    ) {
        viewModelScope.launch {
            try {
                val state = repository.execute(document, command)
                val dropped = annotations.remapPages(remap)
                val note = describe() + if (dropped > 0) " ($dropped mark(s) removed)" else ""
                afterEdit(document, state, note, follow)
            } catch (t: Throwable) {
                reportEditFailure(t)
            }
        }
    }

    /**
     * Bring every index-keyed cache back into line with the edited document.
     *
     * There are five of them, and forgetting one is invisible until a specific
     * page is looked at: the thumbnail rail would keep showing a deleted page, the
     * highlighter would hit-test the wrong page's text, and the raster bridge would
     * flash the old content on the way into zoom. The engine invalidates its own
     * cache from the command's `affected_pages`; everything here is the Kotlin side
     * of the same job.
     */
    private fun afterEdit(
        doc: PdfDocument,
        state: EditState,
        message: String?,
        follow: Int? = null,
    ) {
        thumbnailCache.clear()
        textSegmentCache.clear()
        dropRecentRasters()
        unwarmablePages.clear()

        val pageCount = state.pageCount
        _state.update { current ->
            val page = (follow ?: current.currentPage).coerceIn(0, (pageCount - 1).coerceAtLeast(0))
            current.copy(
                editState = state,
                pageCount = pageCount,
                currentPage = page,
                // Sizes are re-measured below; keeping the old map would leave the
                // layout anchored to pages that have moved.
                pageSizes = emptyMap(),
                pagesWithoutSelectableText = emptySet(),
                jumpToPage = page,
                message = message,
            )
        }
        refreshAnnotations()
        measureAllPages(doc)
        warmThumbnails()
        schedulePrefetch()
    }

    private fun reportEditFailure(t: Throwable) {
        if (t is CancellationException) throw t
        Log.e(TAG, "edit failed", t)
        _state.update { it.copy(message = t.message ?: "That change could not be applied.") }
    }

    // ------------------------------------------------ marks already in the file --

    /**
     * Where a mark loaded from the document lives, by its store id.
     *
     * Erasing one needs two things — the page, and the index PDFium knows it by —
     * and neither is recoverable from the mark once it sits in the store beside
     * marks made this session.
     */
    private val savedMarkLocations = mutableMapOf<Long, Pair<Int, Int>>()

    /** Pages whose saved marks have been read, so they are read once. */
    private val loadedMarkPages = mutableSetOf<Int>()

    /**
     * Load the marks already in the file for one page.
     *
     * Per page rather than for the document on open: a 95-page file would
     * otherwise pay for every page's annotations before drawing the first one.
     */
    fun loadSavedMarks(pageIndex: Int) {
        val doc = document ?: return
        if (!loadedMarkPages.add(pageIndex)) {
            Log.i(TAG, "marks: page=$pageIndex already loaded")
            return
        }

        viewModelScope.launch {
            try {
                val marks = repository.savedMarks(doc, pageIndex) { annotations.nextId() }
                Log.i(TAG, "marks: page=$pageIndex read=${marks.size}")
                if (marks.isEmpty()) return@launch

                for (mark in marks) {
                    // Added without history: these were not made in this session, so
                    // undo must not reach back past the file the reader opened.
                    // Removing one is undoable through the document's own history.
                    annotations.addFromDocument(mark.annotation)
                    savedMarkLocations[mark.annotation.id] = pageIndex to mark.index
                }
                refreshAnnotations()
                SessionRecorder.record("MARKS_LOADED", "page=$pageIndex count=${marks.size}")
            } catch (t: CancellationException) {
                throw t
            } catch (t: Throwable) {
                // The marks still *draw* — they are part of the rendered page — so
                // this is not worth interrupting the reader for. It only means they
                // cannot be erased, and retrying on the next visit is free.
                Log.w(TAG, "could not read marks on page $pageIndex", t)
                loadedMarkPages.remove(pageIndex)
            }
        }
    }

    /**
     * Take out of the file any loaded mark the eraser has just removed.
     *
     * Reconciled by id rather than reported by the eraser, because a single sweep
     * can take several marks and can mix session marks with saved ones; each needs
     * different treatment and this way neither has to know about the other.
     */
    private fun commitErasedSavedMarks(pageIndex: Int) {
        val doc = document ?: return
        val gone = savedMarkLocations
            .filter { (id, at) -> at.first == pageIndex && !annotations.contains(pageIndex, id) }
            .map { (id, at) -> id to at.second }
            // Highest index first. PDFium renumbers everything after a removed
            // annotation, so erasing 1 then 3 in ascending order would take out 1
            // and then whatever had shifted into 3 — a different mark entirely.
            .sortedByDescending { it.second }

        Log.i(TAG, "eraseSaved: page=$pageIndex gone=${gone.size}")
        if (gone.isEmpty()) return
        val removedIndices = gone.map { it.second }

        viewModelScope.launch {
            for ((id, index) in gone) {
                try {
                    repository.execute(doc, PdfCommand.RemoveAnnotation(pageIndex, index))
                    savedMarkLocations.remove(id)
                } catch (t: CancellationException) {
                    throw t
                } catch (t: Throwable) {
                    Log.w(TAG, "could not erase saved mark $id at $index", t)
                    _state.update { it.copy(message = "That mark could not be erased.") }
                }
            }

            // Every surviving mark on this page shifts down by the number of
            // removals below it. Removing highest-first keeps the *removals*
            // correct; it does nothing for the indices left behind, and a stale one
            // would erase the wrong mark next time.
            for ((id, at) in savedMarkLocations.toList()) {
                if (at.first != pageIndex) continue
                val below = removedIndices.count { it < at.second }
                if (below > 0) savedMarkLocations[id] = pageIndex to (at.second - below)
            }

            // The mark was part of the drawn page, so the pixels are now wrong.
            thumbnailCache.clear()
            dropRecentRasters()
            _state.update {
                it.copy(pageContentRevision = it.pageContentRevision + 1)
            }
            refreshEditState()
        }
    }

    private fun refreshEditState() {
        val doc = document ?: return
        viewModelScope.launch {
            runCatching { repository.editState(doc) }
                .getOrNull()
                ?.let { fresh -> _state.update { it.copy(editState = fresh) } }
        }
    }

    // ---------------------------------------------------------------- capture --

    /**
     * Capture what was framed on screen, across however many pages that turns out
     * to be.
     *
     * The engine re-renders each page's share from the document, so the result
     * holds only what is in the PDF — no notification, no dialog of ours, no
     * status bar. That is a consequence of never involving the screen rather than
     * something filtered out afterwards; see roadmap decision 4.8.
     *
     * The tiles come from the reader, the only part of the app that knows where a
     * page sits on a screen. A capture that stopped at the page the drag began on
     * is what made this feel broken: a box drawn across a page join came back
     * holding half of what was inside it.
     *
     * [mask] is the lasso's ring, in capture units and empty for a plain box. It
     * is carried on the request rather than applied once, so re-exporting at 4×
     * keeps the shape someone drew instead of quietly reverting to its box.
     */
    fun capture(
        tiles: List<CaptureTile>,
        area: Rect,
        background: Long,
        originPage: Int,
        mask: List<Offset> = emptyList(),
    ) {
        if (tiles.isEmpty() || !area.isWorthCapturing()) return
        // Remembered before it is overridden, so "the page" is still an
        // answer after another fill has been chosen.
        readerBackground = background
        val existing = _state.value.capture?.request
        takeCapture(
            CaptureRequest(
                tiles = tiles,
                width = area.width,
                height = area.height,
                background = _state.value.captureFill.colour ?: background,
                originPage = originPage,
                mask = mask,
                // Whatever was chosen last, so a second capture does not silently
                // come back at a different resolution from the first.
                scale = existing?.scale ?: CaptureScale.X2,
                format = existing?.format ?: CaptureFormat.PNG,
            ),
        )
    }

    /**
     * Choose what fills the capture where no page reaches.
     *
     * Re-renders what is on screen rather than only applying to the next capture:
     * the fill is a decision about the picture in front of you, and the way to
     * judge it is to see it.
     *
     * Picking [CaptureFill.TRANSPARENT] also moves the export to PNG, because JPEG
     * has no alpha channel: leaving it on JPEG would flatten the cut-out back to a
     * colour and hand back a picture that quietly ignored the choice.
     */
    fun setCaptureFill(fill: CaptureFill) {
        if (_state.value.captureFill == fill) return
        _state.update { it.copy(captureFill = fill) }

        val request = _state.value.capture?.request ?: return
        takeCapture(
            request.copy(
                background = fill.colour ?: readerBackground,
                format = if (fill == CaptureFill.TRANSPARENT) CaptureFormat.PNG else request.format,
            ),
        )
    }

    /**
     * The reader's own backdrop, as of the last capture.
     *
     * Kept so [CaptureFill.PAGE] can be chosen again after another fill has
     * overwritten the request's colour. Only the reader knows this — it comes from
     * the theme — and by the time the editor is open the reader is not on screen
     * to ask.
     */
    private var readerBackground: Long = 0xFFFFFFFFL

    /** Re-render the capture on screen at a different resolution. */
    fun setCaptureScale(scale: CaptureScale) {
        val request = _state.value.capture?.request ?: return
        if (request.scale != scale) takeCapture(request.copy(scale = scale))
    }

    /** Re-render the capture on screen in the other format. */
    fun setCaptureFormat(format: CaptureFormat) {
        val request = _state.value.capture?.request ?: return
        if (request.format != format) takeCapture(request.copy(format = format))
    }

    private fun takeCapture(request: CaptureRequest) {
        val doc = document ?: return
        if (_state.value.isCapturing) return

        _state.update { it.copy(isCapturing = true) }
        viewModelScope.launch {
            try {
                val name = captureFileName(
                    documentName = _state.value.documentName,
                    pageIndex = request.originPage,
                    format = request.format,
                    timestamp = CaptureExport.timestamp(),
                )
                val taken = withContext(Dispatchers.Default) {
                    val bytes = doc.capture(request)
                    CapturePreview(request, bytes, name, decodeForPreview(bytes))
                }
                SessionRecorder.record(
                    kind = "CAPTURE",
                    detail = "page=${request.originPage} pages=${request.tiles.size} " +
                        "scale=${request.scale.label} " +
                        "format=${request.format.wireName} bytes=${taken.bytes.size}",
                )
                _state.update { it.copy(isCapturing = false, capture = taken) }
            } catch (t: Throwable) {
                Log.e(TAG, "capture failed", t)
                _state.update {
                    it.copy(
                        isCapturing = false,
                        message = t.message ?: "That region could not be captured.",
                    )
                }
            }
        }
    }

    /**
     * Decode a copy small enough to show.
     *
     * The preview is a thumbnail on a sheet. Decoding a 4× capture at full size to
     * fill it would allocate tens of megabytes of Java heap for something a few
     * hundred pixels across — on top of the encoded bytes already being held.
     */
    private fun decodeForPreview(bytes: ByteArray): ImageBitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)

        var sample = 1
        while (
            bounds.outWidth / sample > PREVIEW_MAX_EDGE_PX ||
            bounds.outHeight / sample > PREVIEW_MAX_EDGE_PX
        ) {
            sample *= 2
        }

        val decoded = BitmapFactory.decodeByteArray(
            bytes,
            0,
            bytes.size,
            BitmapFactory.Options().apply { inSampleSize = sample },
        ) ?: error("the capture could not be decoded for preview")

        return decoded.asImageBitmap()
    }

    /**
     * Choose between dragging a box and drawing a ring.
     *
     * Remembered rather than reset after each capture: lifting six details
     * off one drawing is the case this tool is for, and re-choosing the shape
     * between each of them is six long presses nobody would forgive.
     */
    fun setCaptureLasso(lasso: Boolean) = _state.update { it.copy(captureLasso = lasso) }

    fun dismissCapture() = _state.update { it.copy(capture = null, markup = emptyList()) }

    // ------------------------------------------------------- markup on a capture --

    fun setMarkupTool(tool: MarkupTool) = _state.update { it.copy(markupTool = tool) }

    fun setMarkupColor(color: Long) = _state.update { it.copy(markupColor = color) }

    /**
     * Choose the line type the next mark is drawn in.
     *
     * It applies to every tool that draws a line, so unlike the tools it sits
     * beside, choosing one does not change what you are drawing with.
     */
    fun setMarkupStyle(style: MarkupStyle) = _state.update { it.copy(markupStyle = style) }

    /**
     * How heavy the current tool draws: nib width, or intensity for the
     * highlighter.
     *
     * Set against the tool it belongs to rather than globally, so each tool keeps
     * whatever it was last set to.
     */
    fun setMarkupSize(tool: MarkupTool, size: Float) = _state.update {
        it.copy(markupSizes = it.markupSizes + (tool to size.coerceIn(tool.sizeRange)))
    }

    /** Add a mark that needed no recognition — a dragged shape, or a plain stroke. */
    fun addMarkup(shape: MarkupShape) = _state.update {
        val tool = it.markupTool
        it.copy(
            markup = it.markup + markupFor(
                shape = shape,
                tool = tool,
                color = it.markupColor,
                size = it.markupSizes[tool] ?: tool.defaultSize,
                style = it.markupStyle,
            ),
        )
    }

    /**
     * Ask the engine what a stroke was, then add whatever it says.
     *
     * Only reached when the finger held still before lifting, so a snap is always
     * something the user asked for. The call itself is pure geometry — no
     * document, no lock — but it still happens after the lift rather than during
     * the drag, so nothing can cost a frame mid-stroke.
     */
    fun recogniseAndAddMarkup(points: List<Offset>) {
        val doc = document ?: return addMarkup(MarkupShape.Freehand(points))
        viewModelScope.launch {
            val shape = withContext(Dispatchers.Default) {
                runCatching { doc.recogniseStroke(points) }
                    .getOrElse { MarkupShape.Freehand(points) }
            }
            addMarkup(shape)
        }
    }

    /**
     * Remove the most recent mark.
     *
     * A snapped shape is one mark, so undoing it removes the whole snap — which is
     * what someone who did not want the shape is reaching for.
     */
    fun undoMarkup() = _state.update {
        it.copy(markup = it.markup.dropLast(1))
    }

    fun clearMarkup() = _state.update { it.copy(markup = emptyList()) }

    /**
     * The bytes to export: the capture with its markup drawn in.
     *
     * Re-rendered rather than taken from the preview, because the preview is the
     * clean capture — the marks on screen are drawn over it by the UI. Rendering
     * again is what makes the file match what is on the sheet, and it is the
     * engine that draws them, so the exported picture is still built from nothing
     * but the document and the committed shapes.
     */
    private suspend fun captureBytesWithMarkup(capture: CapturePreview): ByteArray {
        val marks = _state.value.markup
        if (marks.isEmpty()) return capture.bytes

        val doc = document ?: return capture.bytes
        return withContext(Dispatchers.Default) {
            runCatching { doc.capture(capture.request, marks) }.getOrElse { failure ->
                Log.e(TAG, "drawing the markup failed; exporting the plain capture", failure)
                capture.bytes
            }
        }
    }

    /**
     * The storage permission was refused, below API 29.
     *
     * Says what to do rather than only that it failed: the capture is still on
     * screen, and sharing it needs no permission at all.
     */
    fun noteCaptureNeedsStorage() = _state.update {
        it.copy(message = "Saving to the gallery needs storage access. Share the picture instead.")
    }

    /** Keep the capture in the device's gallery. */
    fun saveCaptureToGallery() {
        val capture = _state.value.capture ?: return
        viewModelScope.launch {
            try {
                val bytes = captureBytesWithMarkup(capture)
                withContext(Dispatchers.IO) {
                    CaptureExport.saveToGallery(
                        context = getApplication(),
                        bytes = bytes,
                        fileName = capture.fileName,
                        format = capture.request.format,
                    )
                }
                _state.update { it.copy(capture = null, message = "Saved to Pictures/Pagify.") }
            } catch (t: Throwable) {
                Log.e(TAG, "saving a capture failed", t)
                _state.update { it.copy(message = captureSaveFailureMessage(t)) }
            }
        }
    }

    /**
     * Below API 29 there is no scoped storage, which makes this the one place in
     * the app that can fail for want of a permission. Naming it is the difference
     * between a user granting it and concluding the feature is broken.
     */
    private fun captureSaveFailureMessage(t: Throwable): String = when (t) {
        is SecurityException -> "Allow storage access to save pictures to the gallery."
        else -> t.message ?: "The picture could not be saved."
    }

    /** Write the capture to the cache and offer it to another app. */
    fun shareCapture() {
        val capture = _state.value.capture ?: return
        viewModelScope.launch {
            try {
                val bytes = captureBytesWithMarkup(capture)
                val uri = withContext(Dispatchers.IO) {
                    CaptureExport.cache(getApplication(), bytes, capture.fileName)
                }
                _state.update {
                    it.copy(captureToShare = CaptureShare(uri, capture.request.format.mimeType))
                }
            } catch (t: Throwable) {
                Log.e(TAG, "sharing a capture failed", t)
                _state.update { it.copy(message = "The picture could not be shared.") }
            }
        }
    }

    fun captureShared() = _state.update { it.copy(captureToShare = null, capture = null) }

    /** Put the capture on the clipboard, for pasting into another app. */
    fun copyCapture() {
        val capture = _state.value.capture ?: return
        viewModelScope.launch {
            try {
                val bytes = captureBytesWithMarkup(capture)
                val uri = withContext(Dispatchers.IO) {
                    CaptureExport.cache(getApplication(), bytes, capture.fileName)
                }
                CaptureExport.copyToClipboard(getApplication(), uri)
                _state.update { it.copy(capture = null, message = "Picture copied.") }
            } catch (t: Throwable) {
                Log.e(TAG, "copying a capture failed", t)
                _state.update { it.copy(message = "The picture could not be copied.") }
            }
        }
    }

    private companion object {
        const val TAG = "PdfReaderViewModel"
        const val DOUBLE_TAP_ZOOM = 2.5f

        /** Enough to bridge a view swap on the current page and its neighbours. */
        const val RECENT_RASTER_COUNT = 4

        /**
         * Byte ceiling for the raster map, whatever the count says.
         *
         * Sized to hold two pages at the 16 MP render ceiling rather than four
         * at whatever size they happen to be. The map only has to bridge a view
         * swap, so two is enough, and the previous count-only bound made this the
         * largest consumer in the app by a wide margin.
         */
        const val RECENT_RASTER_BUDGET_BYTES = 128 * 1024 * 1024

        /** How often the warmer rechecks whether it may resume. */
        const val WARM_YIELD_MILLIS = 80L

        /** Fallback size for an inserted page when nothing has been measured yet. */
        const val A4_WIDTH_POINTS = 595f
        const val A4_HEIGHT_POINTS = 842f

        /**
         * Longest edge of a decoded capture preview, in pixels.
         *
         * The preview fills part of a sheet; 1024 is past what any of our target
         * screens can show of it, and it keeps a 4× capture's decode at a few
         * megabytes rather than tens.
         */
        const val PREVIEW_MAX_EDGE_PX = 1024
    }
}
