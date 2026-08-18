package com.hsilighting.pagify.ui.reader

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.StopCircle
import androidx.compose.material.icons.automirrored.filled.ViewSidebar
import androidx.compose.material.icons.automirrored.filled.RotateRight
import androidx.compose.material.icons.automirrored.filled.Redo
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.core.PenMode
import com.hsilighting.pagify.core.pinchProgressAfter
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.ui.components.AnnotationToolbar
import com.hsilighting.pagify.ui.components.NoTextOnPageHint
import com.hsilighting.pagify.ui.components.annotationLayer
import com.hsilighting.pagify.ui.components.twoFingerPan
import com.hsilighting.pagify.ui.components.PageNavigator
import com.hsilighting.pagify.ui.components.PdfPageView
import com.hsilighting.pagify.ui.components.THUMBNAIL_STRIP_WIDTH
import com.hsilighting.pagify.ui.components.ThumbnailStrip
import com.hsilighting.pagify.ui.components.ViewportWindow
import com.hsilighting.pagify.ui.components.pinchToZoom
import kotlin.math.abs
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PdfReaderScreen(
    state: PdfReaderState,
    onPickDocument: () -> Unit,
    onPageVisible: (Int) -> Unit,
    /**
     * Zoom in from fit-width, pinning the page the gesture landed on, at the given
     * magnification. The zoom is carried so a pinch can hand over at the size it
     * actually reached rather than snapping to a fixed level.
     */
    onZoomInOn: (Int, Float) -> Unit,
    /**
     * A settled zoom level from the pinned view. Reported only once a gesture
     * stops, so prefetching and re-rasterisation do not chase every frame of a
     * pinch. Dropping to fit-width here is what releases the pin.
     */
    onZoomTo: (Float) -> Unit,
    /** Throttled, skippable renderer for thumbnails. See PdfRepository.renderThumbnail. */
    thumbnailRenderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
    /** Viewport width in device pixels, so prefetch can match the render scale. */
    onViewportWidth: (Float) -> Unit,
    onRotate: () -> Unit,
    onToggleThumbnails: () -> Unit,
    /** Start or stop the render-timeline recording. */
    onToggleRecording: () -> Unit,
    /** Fired on every zoom gesture event, to drive the blank-frame watcher. */
    onZoomActivity: () -> Unit,
    /** Reader bounds in window pixels, so the watcher samples only the page area. */
    onContentBounds: (Int, Int, Int, Int) -> Unit,
    /** Synchronous peek at the last raster drawn for a page. */
    peekRenderedPage: (Int) -> android.graphics.Bitmap?,
    // ------------------------------------------------------------ annotation --
    /** Marks already on a page. Re-read whenever `annotationRevision` changes. */
    annotationsForPage: (Int) -> List<Annotation>,
    /** Positioned text for a page; the highlighter hit-tests against this. */
    textSegmentsForPage: suspend (Int) -> List<TextSegment>,
    onAddAnnotation: (Annotation) -> Unit,
    onSelectTool: (AnnotationTool) -> Unit,
    onPenModeChange: (PenMode) -> Unit,
    onPenColorChange: (Long) -> Unit,
    onUndoAnnotation: () -> Unit,
    onRedoAnnotation: () -> Unit,
    /** Open an eraser stroke; everything it takes until [onEraseEnd] is one undo. */
    onEraseStart: () -> Unit,
    /** Rub out whatever is at this page point, within this tolerance in points. */
    onErase: (pageIndex: Int, point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    /** A highlight drag swept this page and selected nothing. */
    onHighlightMissed: (Int) -> Unit,
    onClearPage: (Int) -> Unit,
    onClearAll: () -> Unit,
    /** The reader has taken the scroll a history step asked for. */
    onJumpHandled: () -> Unit,
    onShowMetadata: (Boolean) -> Unit,
    onSubmitPassword: (String) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = state.documentName.ifBlank { "Pagify" },
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                        )
                        if (state.isReady) {
                            Text(
                                text = "Page ${state.currentPageLabel} of ${state.pageCount}",
                                style = MaterialTheme.typography.labelSmall,
                            )
                        }
                    }
                },
                actions = {
                    if (state.isReady) {
                        // Undo and redo sit here rather than in the tool ribbon:
                        // they apply to edits already made, so they have to be
                        // reachable when no tool is selected.
                        IconButton(onClick = onUndoAnnotation, enabled = state.canUndo) {
                            Icon(
                                Icons.AutoMirrored.Filled.Undo,
                                contentDescription = "Undo the last edit",
                            )
                        }
                        IconButton(onClick = onRedoAnnotation, enabled = state.canRedo) {
                            Icon(
                                Icons.AutoMirrored.Filled.Redo,
                                contentDescription = "Redo the last undone edit",
                            )
                        }
                        IconButton(onClick = onToggleRecording) {
                            Icon(
                                imageVector = if (state.isRecording) {
                                    Icons.Filled.StopCircle
                                } else {
                                    Icons.Filled.FiberManualRecord
                                },
                                contentDescription = if (state.isRecording) {
                                    "Stop recording and save the render timeline"
                                } else {
                                    "Record a render timeline"
                                },
                                tint = if (state.isRecording) {
                                    MaterialTheme.colorScheme.error
                                } else {
                                    MaterialTheme.colorScheme.onSurfaceVariant
                                },
                            )
                        }
                        IconButton(onClick = onToggleThumbnails) {
                            Icon(
                                Icons.AutoMirrored.Filled.ViewSidebar,
                                contentDescription = if (state.showThumbnails) {
                                    "Hide page thumbnails"
                                } else {
                                    "Show page thumbnails"
                                },
                                tint = if (state.showThumbnails) {
                                    MaterialTheme.colorScheme.primary
                                } else {
                                    MaterialTheme.colorScheme.onSurfaceVariant
                                },
                            )
                        }
                        IconButton(onClick = onRotate) {
                            Icon(
                                Icons.AutoMirrored.Filled.RotateRight,
                                contentDescription = "Rotate",
                            )
                        }
                        IconButton(onClick = { onShowMetadata(true) }) {
                            Icon(Icons.Filled.Info, contentDescription = "Document details")
                        }
                    }
                    IconButton(onClick = onPickDocument) {
                        Icon(Icons.Filled.FolderOpen, contentDescription = "Open a PDF")
                    }
                },
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when (val phase = state.phase) {
                is PdfReaderState.Phase.Empty -> EmptyState(onPickDocument)

                is PdfReaderState.Phase.Loading -> CircularProgressIndicator(
                    Modifier.align(Alignment.Center),
                )

                is PdfReaderState.Phase.PasswordRequired -> PasswordPrompt(
                    isRetry = phase.retry,
                    onSubmit = onSubmitPassword,
                )

                is PdfReaderState.Phase.Failed -> Message(
                    title = "Could not open this file",
                    detail = phase.message,
                    actionLabel = "Choose another",
                    onAction = onPickDocument,
                )

                is PdfReaderState.Phase.Ready -> PageList(
                    state = state,
                    annotationsForPage = annotationsForPage,
                    textSegmentsForPage = textSegmentsForPage,
                    onAddAnnotation = onAddAnnotation,
                    onEraseStart = onEraseStart,
                    onErase = onErase,
                    onEraseEnd = onEraseEnd,
                    onHighlightMissed = onHighlightMissed,
                    onJumpHandled = onJumpHandled,
                    onPageVisible = onPageVisible,
                    onZoomInOn = onZoomInOn,
                    onZoomTo = onZoomTo,
                    onZoomActivity = onZoomActivity,
                    onContentBounds = onContentBounds,
                    peekRenderedPage = peekRenderedPage,
                    thumbnailRenderer = thumbnailRenderer,
                    onViewportWidth = onViewportWidth,
                    pageSizeProvider = pageSizeProvider,
                    renderer = renderer,
                )
            }

            // Inside the Box so it can align to the bottom, and floating over the
            // reader rather than taking layout space — turning a tool on must not
            // reflow the page you are working on.
            if (state.isReady) {
                // Only while the highlighter is the live tool: the marker, the
                // eraser and plain reading all work perfectly well on a scan, so
                // saying anything then would be noise.
                val highlighterOnScan = state.tool == AnnotationTool.Pen &&
                    state.penMode == PenMode.Highlight &&
                    state.currentPage in state.pagesWithoutSelectableText
                var hintDismissed by remember(state.currentPage) { mutableStateOf(false) }

                // Recorded only while the highlighter is the live tool, which is
                // the only time the answer is interesting — and it is the line
                // that separates "the page has text we failed to find" from "the
                // hint is showing and the reader ignored it".
                val highlighterLive = state.tool == AnnotationTool.Pen &&
                    state.penMode == PenMode.Highlight
                LaunchedEffect(highlighterLive, highlighterOnScan, state.currentPage, hintDismissed) {
                    if (!highlighterLive) return@LaunchedEffect
                    SessionRecorder.record(
                        kind = "SCAN_HINT",
                        detail = "show=$highlighterOnScan dismissed=$hintDismissed " +
                            "page=${state.currentPage} " +
                            "knownScanPages=${state.pagesWithoutSelectableText.size}",
                    )
                }

                if (highlighterOnScan && !hintDismissed) {
                    NoTextOnPageHint(
                        onUseMarker = {
                            onPenModeChange(PenMode.Marker)
                            hintDismissed = true
                        },
                        onDismiss = { hintDismissed = true },
                        modifier = Modifier
                            .align(Alignment.BottomCenter)
                            .padding(bottom = 94.dp),
                    )
                }

                AnnotationToolbar(
                    selectedTool = state.tool,
                    penMode = state.penMode,
                    penColor = state.penColor,
                    onSelectTool = onSelectTool,
                    onPenModeChange = onPenModeChange,
                    onPenColorChange = onPenColorChange,
                    marksOnPage = state.annotationsOnPage,
                    marksInDocument = state.annotationsInDocument,
                    onClearPage = { onClearPage(state.currentPage) },
                    onClearAll = onClearAll,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 20.dp),
                )
            }
        }

        if (state.showMetadataSheet && state.metadata != null) {
            ModalBottomSheet(onDismissRequest = { onShowMetadata(false) }) {
                MetadataSheet(state.metadata)
            }
        }
    }
}

@Composable
private fun PageList(
    state: PdfReaderState,
    annotationsForPage: (Int) -> List<Annotation>,
    textSegmentsForPage: suspend (Int) -> List<TextSegment>,
    onAddAnnotation: (Annotation) -> Unit,
    onEraseStart: () -> Unit,
    onErase: (pageIndex: Int, point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    onHighlightMissed: (Int) -> Unit,
    onJumpHandled: () -> Unit,
    onPageVisible: (Int) -> Unit,
    onZoomInOn: (Int, Float) -> Unit,
    onZoomTo: (Float) -> Unit,
    onZoomActivity: () -> Unit,
    onContentBounds: (Int, Int, Int, Int) -> Unit,
    peekRenderedPage: (Int) -> android.graphics.Bitmap?,
    onViewportWidth: (Float) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
    thumbnailRenderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    val listState = rememberLazyListState()
    val coroutineScope = rememberCoroutineScope()
    val density = LocalDensity.current

    // Where the navigator's viewport indicator comes from while pinned.
    var window by remember { mutableStateOf(ViewportWindow.Full) }
    var recenterRequest by remember { mutableStateOf<Offset?>(null) }

    // Returning to fit-width restores the continuous list at the page that was
    // pinned, so unzooming never drops the reader somewhere else in the document.
    val pinnedPage = state.zoomedPage
    LaunchedEffect(pinnedPage) {
        if (pinnedPage == null) {
            window = ViewportWindow.Full
            listState.scrollToItem(state.currentPage)
        }
    }

    /**
     * Silences page-visibility reports while the reader is being scrolled by us
     * rather than by the user.
     *
     * Choosing a page from the rail centres it, which leaves the *previous* page
     * as the topmost visible one. That reported straight back as a page change,
     * and the rail — which is supposed to hold still when you pick from it —
     * treated the echo as the reader moving and scrolled anyway.
     */
    var scrollingProgrammatically by remember { mutableStateOf(false) }

    /** Bumped only by a reader scroll you performed. The rail keys its follow off this. */
    var readerFollowTick by remember { mutableStateOf(0) }

    // Undo and redo can change a page you are not looking at. Going there is what
    // makes the button's effect visible; without it the edit happens off screen
    // and the button reads as broken.
    LaunchedEffect(state.jumpToPage) {
        val page = state.jumpToPage ?: return@LaunchedEffect
        scrollingProgrammatically = true
        listState.scrollToItem(page)
        delay(SCROLL_SETTLE_MILLIS)
        scrollingProgrammatically = false
        onJumpHandled()
    }

    // Which page you are actually looking at.
    //
    // Not `firstVisibleItemIndex`: at the end of a document the last page can
    // fill the screen while the item *starting* highest is still the one before
    // it, so the counter sat one page short and the rail highlighted the wrong
    // thumbnail. Taking the page nearest the viewport's centre matches what is
    // being read, and pinning to the last item once the list can scroll no
    // further makes the final page reachable at all.
    LaunchedEffect(listState) {
        snapshotFlow {
            val info = listState.layoutInfo
            val items = info.visibleItemsInfo
            when {
                items.isEmpty() -> listState.firstVisibleItemIndex
                !listState.canScrollForward -> items.last().index
                else -> {
                    val centre = (info.viewportStartOffset + info.viewportEndOffset) / 2
                    items.minByOrNull { abs((it.offset + it.size / 2) - centre) }?.index
                        ?: listState.firstVisibleItemIndex
                }
            }
        }
            .distinctUntilChanged()
            .collect { index ->
                if (scrollingProgrammatically) return@collect
                onPageVisible(index)
                // The rail follows this, and only this.
                readerFollowTick++
            }
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val viewportWidth = maxWidth
        val viewportHeight = maxHeight

        // The prefetcher needs this to warm the cache at the scale the page views
        // will actually request.
        LaunchedEffect(viewportWidth, density) {
            onViewportWidth(with(density) { viewportWidth.toPx() })
        }

        // The width the list draws a page at. Shared with the pinned view so that
        // scale 1.0 means the same thing on both sides of the handover.
        val railWidthForBase = if (state.showThumbnails) THUMBNAIL_STRIP_WIDTH else 0.dp
        val listPageWidthPx = with(density) {
            (viewportWidth - railWidthForBase - PAGE_GAP * 2).toPx()
        }

        if (pinnedPage != null) {
            // The highlighter needs the page's text runs here just as it does in
            // the list. Keyed on the tool as well as the page so switching to the
            // highlighter while already magnified loads them rather than waiting
            // for the page to change.
            val wantsText = state.tool == AnnotationTool.Pen &&
                state.penMode == PenMode.Highlight
            var pinnedSegments by remember(pinnedPage) {
                mutableStateOf<List<TextSegment>>(emptyList())
            }
            LaunchedEffect(pinnedPage, wantsText) {
                if (wantsText && pinnedSegments.isEmpty()) {
                    pinnedSegments = textSegmentsForPage(pinnedPage)
                }
            }

            // Zoomed: one page, both axes pannable, bounded by that page.
            ZoomedPage(
                pageIndex = pinnedPage,
                initialZoom = state.zoom,
                pageSize = state.pageSizes[pinnedPage],
                onZoomSettled = onZoomTo,
                onZoomActivity = onZoomActivity,
                onWindowChanged = { window = it },
                pageSizeProvider = pageSizeProvider,
                renderer = renderer,
                initialBitmap = peekRenderedPage(pinnedPage),
                basePageWidthPx = listPageWidthPx,
                initialFocus = recenterRequest,
                annotations = remember(pinnedPage, state.annotationRevision) {
                    annotationsForPage(pinnedPage)
                },
                textSegments = pinnedSegments,
                tool = state.tool,
                penMode = state.penMode,
                penColor = state.penColor,
                onAddAnnotation = onAddAnnotation,
                onEraseStart = onEraseStart,
                onErase = { point, tolerance -> onErase(pinnedPage, point, tolerance) },
                onEraseEnd = onEraseEnd,
            )
        } else {
            val viewportWidthPx = with(density) { viewportWidth.toPx() }

            /**
             * Which page a touch landed on, and where within it as a 0..1 fraction.
             *
             * Needed because the zoom that *enters* pinned mode originates here,
             * where `ZoomedPage` does not exist yet to capture a focal point. The
             * fraction is handed over as the initial recentre so the zoom lands on
             * what was touched instead of the page's top-left corner.
             */
            fun focusAt(position: Offset): Pair<Int, Offset>? {
                val item = listState.layoutInfo.visibleItemsInfo.firstOrNull { info ->
                    position.y >= info.offset && position.y < info.offset + info.size
                } ?: return null
                if (item.size <= 0 || viewportWidthPx <= 0f) return null
                return item.index to Offset(
                    (position.x / viewportWidthPx).coerceIn(0f, 1f),
                    ((position.y - item.offset) / item.size).coerceIn(0f, 1f),
                )
            }

            fun enterZoom(position: Offset, targetZoom: Float) {
                val (page, fraction) = focusAt(position) ?: return
                recenterRequest = fraction
                onZoomInOn(page, targetZoom)
            }

            /**
             * Running product of an in-progress pinch, before the pinned view exists.
             *
             * The pinned view is a different composable, so handing over ends the
             * gesture Compose is delivering here — the rest of the pinch is not
             * received. Handing over on the very first event therefore meant one
             * tiny movement decided the whole zoom, and it was answered with a
             * fixed 2.5x jump regardless of how far the fingers had actually moved.
             *
             * Accumulating instead lets the gesture speak: the handover happens once
             * the pinch is unambiguous, and it carries the magnification reached by
             * that point rather than a constant.
             *
             * It is reset when the fingers lift, and **not** clamped as it goes.
             * Clamping each step at 1.0 made this a ratchet: the wobble of a
             * two-finger scroll pushed it up and down in equal measure, but only
             * the upward half survived the clamp, so it crept towards the handover
             * threshold and eventually zoomed the reader in on its own. That is
             * also why it only ever went *in*. The clamp now applies to the value
             * handed over, which is the only place it was needed.
             */
            var pinchProgress by remember { mutableStateOf(1f) }

            // Only pages you have actually landed on get a readable render;
            // everything you flick past stays on its cheap proxy.
            val settled = !listState.isScrollInProgress

            Row(Modifier.fillMaxSize()) {
                if (state.showThumbnails) {
                    ThumbnailStrip(
                        pageCount = state.pageCount,
                        currentPage = state.currentPage,
                        followTick = readerFollowTick,
                        onSelectPage = { page ->
                            coroutineScope.launch {
                                scrollingProgrammatically = true

                                // Centre the chosen page rather than pinning it to
                                // the top: a spread you picked deliberately should
                                // land where you are looking, with its neighbours
                                // visible either side.
                                //
                                // The size has to be *fetched*, not read from
                                // state.pageSizes. That map only holds pages the
                                // reader has already measured, so picking a page
                                // from the rail that had not been scrolled to left
                                // it null, the offset silently fell back to 0, and
                                // the page pinned to the top — the exact symptom
                                // reported. Measuring is cheap now (page_size does
                                // not load the page), so it is simply awaited.
                                val size = state.pageSizes[page] ?: pageSizeProvider(page)

                                val width = viewportWidth -
                                    (if (state.showThumbnails) THUMBNAIL_STRIP_WIDTH else 0.dp) -
                                    PAGE_GAP * 2
                                val pageHeightPx = size?.takeIf { it.aspectRatio > 0f }?.let {
                                    with(density) { width.toPx() } / it.aspectRatio
                                }
                                val viewportHeightPx = with(density) { viewportHeight.toPx() }

                                // A page taller than the viewport cannot be centred;
                                // aligning its top is the closest thing to it.
                                //
                                // The parentheses matter. Written as
                                // `-(...).toInt().coerceAtMost(0)` the clamp binds
                                // before the negation, so the positive half-gap is
                                // clamped to zero and every page lands at the top —
                                // which is exactly what it did.
                                val offset = pageHeightPx?.let {
                                    val centred = -(((viewportHeightPx - it) / 2f).toInt())
                                    centred.coerceAtMost(0)
                                } ?: 0

                                SessionRecorder.record(
                                    kind = "RAIL_SELECT",
                                    detail = "page=$page sizeKnown=${size != null} " +
                                        "pageH=${pageHeightPx?.toInt() ?: -1} " +
                                        "viewportH=${viewportHeightPx.toInt()} offset=$offset",
                                )

                                listState.scrollToItem(page, offset)

                                SessionRecorder.record(
                                    kind = "RAIL_LANDED",
                                    detail = "page=$page firstVisible=${listState.firstVisibleItemIndex} " +
                                        "itemOffset=${listState.firstVisibleItemScrollOffset}",
                                )

                                // Long enough for the settled position to be
                                // observed and discarded by the collector above.
                                delay(SCROLL_SETTLE_MILLIS)

                                // Logged *after* the settle, because RAIL_LANDED
                                // is measured the instant the scroll returns and
                                // therefore cannot see anything that moves the list
                                // afterwards — which is exactly the failure being
                                // chased: pages between here and the target are
                                // composed at a guessed aspect and then resize as
                                // their real dimensions arrive, dragging the anchor.
                                SessionRecorder.record(
                                    kind = "RAIL_SETTLED",
                                    detail = "page=$page firstVisible=${listState.firstVisibleItemIndex} " +
                                        "itemOffset=${listState.firstVisibleItemScrollOffset} " +
                                        "drifted=${listState.firstVisibleItemIndex != (page - 1).coerceAtLeast(0)}",
                                )

                                scrollingProgrammatically = false
                            }
                            onPageVisible(page)
                        },
                        pageSizeProvider = pageSizeProvider,
                        renderer = thumbnailRenderer,
                    )
                }

                Box(
                    Modifier
                        .weight(1f)
                        .fillMaxHeight()
                        // Tells the blank watcher exactly which pixels are meant
                        // to be showing a page — the rail must stay out of it.
                        .onGloballyPositioned { coordinates ->
                            val bounds = coordinates.boundsInWindow()
                            onContentBounds(
                                bounds.left.toInt(),
                                bounds.top.toInt(),
                                bounds.right.toInt(),
                                bounds.bottom.toInt(),
                            )
                        }
                        .pinchToZoom(onGestureEnd = { pinchProgress = 1f }) { factor, centroid ->
                            onZoomActivity()
                            pinchProgress = pinchProgressAfter(pinchProgress, factor)
                            if (pinchProgress >= PINCH_HANDOVER_ZOOM) {
                                // Clamped only here: pinching *out* at fit-width
                                // has nowhere to go, and must not hand over below
                                // the size the reader is already showing.
                                enterZoom(centroid, pinchProgress.coerceAtLeast(1f))
                                pinchProgress = 1f
                            }
                        }
                        .pointerInput(Unit) {
                            detectTapGestures(
                                onDoubleTap = {
                                    onZoomActivity()
                                    // A double-tap is a request for a specific
                                    // magnification, so the jump here is the point.
                                    enterZoom(it, DOUBLE_TAP_ZOOM)
                                },
                            )
                        },
                ) {
                    // The rail takes its width out of the reader, so pages must be
                    // measured against what is left, not the whole viewport.
                    val railWidth = if (state.showThumbnails) THUMBNAIL_STRIP_WIDTH else 0.dp
                    val pageWidth = viewportWidth - railWidth - PAGE_GAP * 2

                    // With a tool live, one finger belongs to the tool. Leaving the
                    // list scrollable meant a highlight drag raced the scroller and
                    // usually lost, since the scroll container claims the gesture
                    // once past touch slop. Two fingers pan instead, handled below.
                    val toolActive = state.tool != AnnotationTool.None

                    LazyColumn(
                        state = listState,
                        modifier = Modifier
                            .fillMaxSize()
                            // Two-finger scrolling, whether or not a tool is live.
                            // It has to be handled here in both cases: the pinch
                            // handler claims every two-finger event on the Initial
                            // pass, so the list's own scrolling never sees one.
                            // Previously this was fitted only when a tool was
                            // active, which left two fingers doing nothing at all
                            // the rest of the time — and, before the gate in
                            // `pinchToZoom`, slowly zooming instead.
                            .twoFingerPan { delta ->
                                coroutineScope.launch { listState.scrollBy(-delta) }
                            },
                        userScrollEnabled = !toolActive,
                        contentPadding = PaddingValues(PAGE_GAP),
                        verticalArrangement = Arrangement.spacedBy(PAGE_GAP),
                    ) {
                        items(count = state.pageCount) { index ->
                            AnnotatablePage(
                                pageIndex = index,
                                pageWidth = pageWidth,
                                readable = settled,
                                state = state,
                                annotationsForPage = annotationsForPage,
                                textSegmentsForPage = textSegmentsForPage,
                                onAddAnnotation = onAddAnnotation,
                                onEraseStart = onEraseStart,
                                onErase = onErase,
                                onEraseEnd = onEraseEnd,
                                onHighlightMissed = onHighlightMissed,
                                pageSizeProvider = pageSizeProvider,
                                renderer = renderer,
                            )
                        }
                    }
                }
            }
        }

        // ------------------------------------------------------------ navigator --
        if (pinnedPage != null && !window.coversEverything) {
            PageNavigator(
                pageIndex = pinnedPage,
                pageSize = state.pageSizes[pinnedPage],
                window = window,
                thumbnailRenderer = thumbnailRenderer,
                modifier = Modifier.align(Alignment.CenterEnd),
                onRecenter = { fx, fy -> recenterRequest = Offset(fx, fy) },
            )
        }
    }
}

/**
 * A page with its annotations drawn over it, and pen input when a tool is on.
 *
 * The render scale is derived here rather than passed down, because the layer
 * has to convert page points to pixels using the *same* factor the page itself
 * was drawn at — otherwise a mark sits slightly off the text it belongs to.
 */
@Composable
private fun AnnotatablePage(
    pageIndex: Int,
    pageWidth: Dp,
    readable: Boolean,
    state: PdfReaderState,
    annotationsForPage: (Int) -> List<Annotation>,
    textSegmentsForPage: suspend (Int) -> List<TextSegment>,
    onAddAnnotation: (Annotation) -> Unit,
    onEraseStart: () -> Unit,
    onErase: (pageIndex: Int, point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    onHighlightMissed: (Int) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    val density = LocalDensity.current
    val pageSize = state.pageSizes[pageIndex]

    // Pixels per page point, at the width this page is actually drawn.
    val renderScale = remember(pageSize, pageWidth, density) {
        val size = pageSize ?: return@remember 0f
        if (size.widthPoints <= 0f) return@remember 0f
        with(density) { pageWidth.toPx() } / size.widthPoints
    }

    // Only loaded when the highlighter is actually in use: walking every text run
    // on a page is real work, and no other tool needs it.
    var segments by remember(pageIndex) { mutableStateOf<List<TextSegment>>(emptyList()) }
    val wantsText = state.tool == AnnotationTool.Pen && state.penMode == PenMode.Highlight
    LaunchedEffect(pageIndex, wantsText) {
        if (wantsText && segments.isEmpty()) segments = textSegmentsForPage(pageIndex)
    }

    // Re-read through the revision counter: the store is mutable and identity
    // stable, so Compose cannot otherwise see that a mark was added.
    val annotations = remember(pageIndex, state.annotationRevision) {
        annotationsForPage(pageIndex).also {
            SessionRecorder.record(
                kind = "PAGE_MARKS",
                detail = "page=$pageIndex rev=${state.annotationRevision} " +
                    "drawing=${it.size} canUndo=${state.canUndo}",
            )
        }
    }

    PdfPageView(
        pageIndex = pageIndex,
        pageWidth = pageWidth,
        readable = readable,
        knownSize = pageSize,
        pageSizeProvider = pageSizeProvider,
        renderer = renderer,
        modifier = Modifier.annotationLayer(
            pageIndex = pageIndex,
            annotations = annotations,
            textSegments = segments,
            tool = state.tool,
            penMode = state.penMode,
            penColor = state.penColor,
            renderScale = renderScale,
            onAdd = onAddAnnotation,
            onRequestNote = { /* Note tool is wired in the next slice. */ },
            onEraseStart = onEraseStart,
            onErase = { point, tolerance -> onErase(pageIndex, point, tolerance) },
            onEraseEnd = onEraseEnd,
            onHighlightMissed = { onHighlightMissed(pageIndex) },
        ),
    )
}

@Composable
private fun EmptyState(onPickDocument: () -> Unit) = Message(
    title = "No document open",
    detail = "Choose a PDF to start reading.",
    actionLabel = "Open a PDF",
    onAction = onPickDocument,
)

@Composable
private fun Message(title: String, detail: String, actionLabel: String, onAction: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(title, style = MaterialTheme.typography.titleLarge, textAlign = TextAlign.Center)
        Text(
            detail,
            style = MaterialTheme.typography.bodyMedium,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 8.dp, bottom = 24.dp),
        )
        Button(onClick = onAction) { Text(actionLabel) }
    }
}

@Composable
private fun PasswordPrompt(isRetry: Boolean, onSubmit: (String) -> Unit) {
    var password by remember { mutableStateOf("") }

    Column(
        modifier = Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("This document is protected", style = MaterialTheme.typography.titleLarge)
        Text(
            text = if (isRetry) "That password was not accepted." else "Enter its password to continue.",
            style = MaterialTheme.typography.bodyMedium,
            color = if (isRetry) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 8.dp, bottom = 16.dp),
        )
        OutlinedTextField(
            value = password,
            onValueChange = { password = it },
            label = { Text("Password") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
        )
        Button(
            onClick = { onSubmit(password) },
            enabled = password.isNotEmpty(),
            modifier = Modifier.padding(top = 16.dp),
        ) { Text("Unlock") }
    }
}

@Composable
private fun MetadataSheet(metadata: PdfMetadata) {
    Column(Modifier.padding(horizontal = 24.dp).padding(bottom = 32.dp)) {
        Text("Document details", style = MaterialTheme.typography.titleLarge)
        Column(Modifier.padding(top = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            MetadataRow("Title", metadata.title)
            MetadataRow("Author", metadata.author)
            MetadataRow("Subject", metadata.subject)
            MetadataRow("Keywords", metadata.keywords)
            MetadataRow("Creator", metadata.creator)
            MetadataRow("Producer", metadata.producer)
            MetadataRow("Pages", metadata.pageCount.toString())
        }
    }
}

@Composable
private fun MetadataRow(label: String, value: String?) {
    if (value.isNullOrBlank()) return
    Column {
        Text(label, style = MaterialTheme.typography.labelMedium)
        Text(value, style = MaterialTheme.typography.bodyLarge)
    }
}

private val PAGE_GAP = 12.dp

/**
 * Magnification at which an in-progress pinch hands over to the pinned view.
 *
 * Low enough that the handover feels like a continuation of the gesture rather
 * than a jump, high enough that an incidental two-finger touch while scrolling
 * does not trip it.
 */
private const val PINCH_HANDOVER_ZOOM = 1.12f

/** Where a double-tap lands. Mirrors the ViewModel's own constant. */
private const val DOUBLE_TAP_ZOOM = 2.5f

/** How long to ignore visibility echoes after scrolling the reader ourselves. */
private const val SCROLL_SETTLE_MILLIS = 250L

