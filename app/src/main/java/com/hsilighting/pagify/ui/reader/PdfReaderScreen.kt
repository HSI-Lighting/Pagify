package com.hsilighting.pagify.ui.reader

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.gestures.detectTapGestures
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
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.ui.components.PageNavigator
import com.hsilighting.pagify.ui.components.PdfPageView
import com.hsilighting.pagify.ui.components.THUMBNAIL_STRIP_WIDTH
import com.hsilighting.pagify.ui.components.ThumbnailStrip
import com.hsilighting.pagify.ui.components.ViewportWindow
import com.hsilighting.pagify.ui.components.pinchToZoom
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

    // Reporting the first *visible* item (rather than the centred one) keeps the
    // page counter in step with what the user sees while scrolling.
    LaunchedEffect(listState) {
        snapshotFlow { listState.firstVisibleItemIndex }
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
             * that point rather than a constant. Clamped at 1.0 so pinching *out* at
             * fit-width, where there is nowhere to go, never banks negative progress
             * that a later pinch would have to undo.
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
                                // sit in the middle of the reader, with its
                                // neighbours visible either side.
                                val size = state.pageSizes[page]
                                val pageHeightPx = size?.let {
                                    val width = viewportWidth -
                                        (if (state.showThumbnails) THUMBNAIL_STRIP_WIDTH else 0.dp) -
                                        PAGE_GAP * 2
                                    if (it.aspectRatio > 0f) {
                                        with(density) { width.toPx() } / it.aspectRatio
                                    } else {
                                        null
                                    }
                                }
                                val viewportHeightPx = with(density) { viewportHeight.toPx() }
                                val offset = pageHeightPx
                                    ?.let { -((viewportHeightPx - it) / 2f).toInt() }
                                    ?: 0
                                listState.scrollToItem(page, offset)
                                // Long enough for the settled position to be
                                // observed and discarded by the collector above.
                                delay(SCROLL_SETTLE_MILLIS)
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
                        .pinchToZoom { factor, centroid ->
                            onZoomActivity()
                            pinchProgress = (pinchProgress * factor).coerceAtLeast(1f)
                            if (pinchProgress >= PINCH_HANDOVER_ZOOM) {
                                enterZoom(centroid, pinchProgress)
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

                    LazyColumn(
                        state = listState,
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(PAGE_GAP),
                        verticalArrangement = Arrangement.spacedBy(PAGE_GAP),
                    ) {
                        items(count = state.pageCount) { index ->
                            PdfPageView(
                                pageIndex = index,
                                pageWidth = pageWidth,
                                readable = settled,
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

