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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.Info
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
    /** Multiply the current zoom, which is what a pinch gesture reports. */
    onZoomBy: (Float) -> Unit,
    /** Double-tap: jump between fit-width and a readable zoom. */
    onToggleZoom: () -> Unit,
    /** Viewport width in device pixels, so prefetch can match the render scale. */
    onViewportWidth: (Float) -> Unit,
    onRotate: () -> Unit,
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
                    onZoomBy = onZoomBy,
                    onToggleZoom = onToggleZoom,
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
    onZoomBy: (Float) -> Unit,
    onToggleZoom: () -> Unit,
    onViewportWidth: (Float) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    val listState = rememberLazyListState()
    val horizontalScroll = rememberScrollState()
    val coroutineScope = rememberCoroutineScope()
    val density = LocalDensity.current

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

        // Zoom is applied as real layout width rather than a graphicsLayer scale.
        // That costs a relayout per zoom step, but it means the page is
        // re-rasterised at its new size (sharp, not an upscaled bitmap) and the
        // stock scroll containers handle panning, flinging and clamping for free.
        val pageWidth = viewportWidth * state.zoom

        Box(
            Modifier
                .fillMaxSize()
                .horizontalScroll(horizontalScroll)
                .pinchToZoom(onZoomBy)
                .pointerInput(Unit) {
                    detectTapGestures(onDoubleTap = { onToggleZoom() })
                },
        ) {
            LazyColumn(
                state = listState,
                modifier = Modifier
                    .width(pageWidth + PAGE_GAP * 2)
                    .fillMaxHeight(),
                contentPadding = PaddingValues(PAGE_GAP),
                verticalArrangement = Arrangement.spacedBy(PAGE_GAP),
            ) {
                items(count = state.pageCount) { index ->
                    PdfPageView(
                        pageIndex = index,
                        pageWidth = pageWidth,
                        pageSizeProvider = pageSizeProvider,
                        renderer = renderer,
                    )
                }
            }
        }

        // ------------------------------------------------------------ navigator --
        val currentPageSize = state.pageSizes[state.currentPage]
        val window = rememberViewportWindow(
            listState = listState,
            horizontalScroll = horizontalScroll,
            currentPage = state.currentPage,
            pageSize = currentPageSize,
            viewportHeight = viewportHeight,
            pageWidth = pageWidth,
        )

        if (!window.coversEverything) {
            PageNavigator(
                pageIndex = state.currentPage,
                pageSize = currentPageSize,
                window = window,
                thumbnailRenderer = renderer,
                modifier = Modifier.align(Alignment.CenterEnd),
                onRecenter = { fx, fy ->
                    coroutineScope.launch {
                        recenterViewport(
                            fractionX = fx,
                            fractionY = fy,
                            listState = listState,
                            horizontalScroll = horizontalScroll,
                            currentPage = state.currentPage,
                            pageHeightPx = pageHeightPx(currentPageSize, pageWidth, density),
                            viewportHeightPx = with(density) { viewportHeight.toPx() },
                        )
                    }
                },
            )
        }
    }
}

private fun pageHeightPx(pageSize: PageSize?, pageWidth: Dp, density: Density): Float {
    val aspect = pageSize?.aspectRatio ?: (595f / 842f)
    if (aspect <= 0f) return 0f
    return with(density) { pageWidth.toPx() } / aspect
}

/**
 * Derive which fraction of the current page is on screen.
 *
 * Horizontal comes from the scroll container's own range. Vertical comes from
 * `LazyListState`: the visible window starts at the current page's scroll offset
 * and is one viewport tall. Both are expressed as 0..1 fractions so the navigator
 * stays ignorant of zoom and density.
 */
@Composable
private fun rememberViewportWindow(
    listState: LazyListState,
    horizontalScroll: ScrollState,
    currentPage: Int,
    pageSize: PageSize?,
    viewportHeight: Dp,
    pageWidth: Dp,
): ViewportWindow {
    val density = LocalDensity.current
    // Reads observable scroll state, so this recomputes as the user pans.
    val scrollOffset = horizontalScroll.value
    val scrollMax = horizontalScroll.maxValue
    val firstIndex = listState.firstVisibleItemIndex
    val firstOffset = listState.firstVisibleItemScrollOffset

    return remember(
        scrollOffset, scrollMax, firstIndex, firstOffset,
        currentPage, pageSize, viewportHeight, pageWidth, density,
    ) {
        val pageHeight = pageHeightPx(pageSize, pageWidth, density)
        val viewportPx = with(density) { viewportHeight.toPx() }
        if (pageHeight <= 0f || viewportPx <= 0f) return@remember ViewportWindow.Full

        val contentWidthPx = with(density) { pageWidth.toPx() }
        val viewportWidthPx = contentWidthPx - scrollMax

        val widthFraction = (viewportWidthPx / contentWidthPx).coerceIn(0f, 1f)
        val leftFraction = if (scrollMax <= 0) 0f else {
            (scrollOffset.toFloat() / contentWidthPx).coerceIn(0f, 1f - widthFraction)
        }

        // Only meaningful while the current page is the one at the top of the list;
        // mid-transition between pages the window is reported as full rather than
        // guessed at.
        val topFraction = if (firstIndex == currentPage) {
            (firstOffset / pageHeight).coerceIn(0f, 1f)
        } else {
            0f
        }
        val heightFraction = (viewportPx / pageHeight).coerceIn(0f, 1f)

        ViewportWindow(
            left = leftFraction,
            top = topFraction.coerceAtMost(1f - heightFraction),
            width = widthFraction,
            height = heightFraction,
        )
    }
}

/** Centre the viewport on a point the user picked in the navigator. */
private suspend fun recenterViewport(
    fractionX: Float,
    fractionY: Float,
    listState: LazyListState,
    horizontalScroll: ScrollState,
    currentPage: Int,
    pageHeightPx: Float,
    viewportHeightPx: Float,
) {
    val targetX = (fractionX * horizontalScroll.maxValue).toInt()
    horizontalScroll.scrollTo(targetX.coerceIn(0, horizontalScroll.maxValue))

    if (pageHeightPx > 0f) {
        val targetY = (fractionY * pageHeightPx - viewportHeightPx / 2f)
            .coerceIn(0f, (pageHeightPx - viewportHeightPx).coerceAtLeast(0f))
        listState.scrollToItem(currentPage, targetY.toInt())
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

