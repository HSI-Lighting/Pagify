package com.hsilighting.pagify.ui.reader

import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.ui.components.PdfPageView
import kotlinx.coroutines.flow.distinctUntilChanged

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PdfReaderScreen(
    state: PdfReaderState,
    onPickDocument: () -> Unit,
    onPageVisible: (Int) -> Unit,
    onZoomChange: (Float) -> Unit,
    onRotate: () -> Unit,
    onShowMetadata: (Boolean) -> Unit,
    onSubmitPassword: (String) -> Unit,
    pageSizeProvider: suspend (Int) -> com.hsilighting.pagify.core.PageSize?,
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
                    onZoomChange = onZoomChange,
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
    onZoomChange: (Float) -> Unit,
    pageSizeProvider: suspend (Int) -> com.hsilighting.pagify.core.PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    val listState = rememberLazyListState()

    // Reporting the first *visible* item (rather than the centred one) keeps the
    // page counter in step with what the user sees while scrolling.
    LaunchedEffect(listState) {
        snapshotFlow { listState.firstVisibleItemIndex }
            .distinctUntilChanged()
            .collect(onPageVisible)
    }

    BoxWithConstraints(
        Modifier
            .fillMaxSize()
            .pointerInput(Unit) {
                detectTransformGestures { _, _, gestureZoom, _ ->
                    if (gestureZoom != 1f) onZoomChange(state.zoom * gestureZoom)
                }
            },
    ) {
        val availableWidth = maxWidth

        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(PAGE_GAP),
            verticalArrangement = Arrangement.spacedBy(PAGE_GAP),
        ) {
            items(count = state.pageCount) { index ->
                PdfPageView(
                    pageIndex = index,
                    zoom = state.zoom,
                    containerWidth = availableWidth,
                    pageSizeProvider = pageSizeProvider,
                    renderer = renderer,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
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
