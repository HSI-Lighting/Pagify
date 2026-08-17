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
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PdfReaderScreen(
    state: PdfReaderState,
    onPickDocument: () -> Unit,
    onPageVisible: (Int) -> Unit,
    /** Zoom in from fit-width, pinning the page the gesture landed on. */
    onZoomInOn: (Int) -> Unit,
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
    /** Zoom in from fit-width, pinning the page the gesture landed on. */
    onZoomInOn: (Int) -> Unit,
    onZoomTo: (Float) -> Unit,
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

    // Reporting the first *visible* item (rather than the centred one) keeps the
    // page counter in step with what the user sees while scrolling.
    LaunchedEffect(listState) {
        snapshotFlow { listState.firstVisibleItemIndex }
            .distinctUntilChanged()
            .collect(onPageVisible)
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val viewportWidth = maxWidth
        val viewportHeight = maxHeight

        // The prefetcher needs this to warm the cache at the scale the page views
        // will actually request.
        LaunchedEffect(viewportWidth, density) {
            onViewportWidth(with(density) { viewportWidth.toPx() })
        }

        if (pinnedPage != null) {
            // Zoomed: one page, both axes pannable, bounded by that page.
            ZoomedPage(
                pageIndex = pinnedPage,
                initialZoom = state.zoom,
                pageSize = state.pageSizes[pinnedPage],
                onZoomSettled = onZoomTo,
                onWindowChanged = { window = it },
                pageSizeProvider = pageSizeProvider,
                renderer = renderer,
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

            fun enterZoom(position: Offset) {
                val (page, fraction) = focusAt(position) ?: return
                recenterRequest = fraction
                onZoomInOn(page)
            }

            // Only pages you have actually landed on get a readable render;
            // everything you flick past stays on its cheap proxy.
            val settled = !listState.isScrollInProgress

            Row(Modifier.fillMaxSize()) {
                if (state.showThumbnails) {
                    ThumbnailStrip(
                        pageCount = state.pageCount,
                        currentPage = state.currentPage,
                        onSelectPage = { page ->
                            coroutineScope.launch { listState.scrollToItem(page) }
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
                        .pinchToZoom { _, centroid -> enterZoom(centroid) }
                        .pointerInput(Unit) {
                            detectTapGestures(onDoubleTap = { enterZoom(it) })
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

