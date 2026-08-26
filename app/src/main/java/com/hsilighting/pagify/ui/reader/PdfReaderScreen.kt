package com.hsilighting.pagify.ui.reader

import androidx.compose.ui.text.style.TextOverflow
import com.hsilighting.pagify.ui.components.PagePicker
import androidx.compose.material.icons.filled.FileUpload
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.foundation.ScrollState
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
import androidx.compose.material.icons.filled.AddPhotoAlternate
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.GridView
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
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
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
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import com.hsilighting.pagify.ui.components.ReaderAction
import com.hsilighting.pagify.ui.components.ReaderActionBar
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PageMapping
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.core.pinchProgressAfter
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.ui.components.AnnotationToolbar
import com.hsilighting.pagify.ui.components.CaptureHint
import com.hsilighting.pagify.ui.components.CaptureEditor
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.CaptureTile
import com.hsilighting.pagify.core.PlacedPage
import com.hsilighting.pagify.core.captureMaskFor
import com.hsilighting.pagify.core.captureTilesFor
import com.hsilighting.pagify.ui.components.captureOverlay
import com.hsilighting.pagify.ui.components.doubleTapToZoom
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.defaultSize
import com.hsilighting.pagify.ui.components.NoTextOnPageHint
import com.hsilighting.pagify.ui.components.NoteComposer
import com.hsilighting.pagify.ui.components.TextComposer
import com.hsilighting.pagify.ui.components.NoteReader
import com.hsilighting.pagify.ui.components.RecognisingTextHint
import com.hsilighting.pagify.ui.components.SaveAction
import com.hsilighting.pagify.ui.components.TextSelectionBar
import com.hsilighting.pagify.ui.components.annotationLayer
import com.hsilighting.pagify.ui.components.twoFingerPan
import com.hsilighting.pagify.ui.components.PageAction
import com.hsilighting.pagify.ui.components.PageNavigator
import com.hsilighting.pagify.ui.components.BlankPageSheet
import com.hsilighting.pagify.ui.components.BlankSheet
import com.hsilighting.pagify.ui.components.PageOrganiser
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
    /** The window is too narrow to give a quarter of it to the thumbnail rail. */
    onNarrowScreen: () -> Unit,
    /** Whether the viewfinder may appear at all, and whether it is folded away. */
    showViewfinder: Boolean,
    viewfinderMinimized: Boolean,
    onViewfinderMinimized: (Boolean) -> Unit,
    viewfinderHandle: Offset,
    onViewfinderHandleMoved: (Offset) -> Unit,
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
    /** The Note tool was tapped at this page point; ask the reader for the text. */
    /** The reader typed a note, or dismissed the composer. */
    onConfirmNote: (String) -> Unit,
    onCancelNote: () -> Unit,
    /** A note marker was tapped, and what to do with the note it opened. */
    onOpenNote: (com.hsilighting.pagify.core.Annotation.Note) -> Unit,
    onCloseNote: () -> Unit,
    onDeleteNote: () -> Unit,
    onRequestNote: (pageIndex: Int, anchor: Offset) -> Unit,
    /** This page is on screen; load any marks the file already holds for it. */
    onPageMarksNeeded: (Int) -> Unit,
    onSelectTool: (AnnotationTool) -> Unit,
    /** How heavy the drawing tools are, and what kind of line they draw. */
    onStrokeWidth: (Float) -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    /** What text is written in and how big, and the words themselves. */
    onTextFont: (PdfFont) -> Unit,
    onTextSize: (Float) -> Unit,
    /** How far a curved caption bends, end to end, in degrees. */
    onTextCurve: (Float) -> Unit,
    onPlaceText: (pageIndex: Int, path: List<Offset>) -> Unit,
    /** Placed text was dragged to a new spot on the same page. */
    onMoveText: (id: Long, delta: Offset) -> Unit,
    /** A caption was tapped, so the ribbon's controls now belong to it. */
    onSelectText: (id: Long?) -> Unit,
    /** Two fingers with a caption in hand: that big. */
    onScaleText: (factor: Float) -> Unit,
    /** Swiped past the end of a magnified page; move to the next or previous. */
    onTurnZoomedPage: (delta: Int) -> Boolean,
    /** A caption was double-tapped; rewrite its words. */
    onEditText: (id: Long) -> Unit,
    onCommitText: (String) -> Unit,
    onCancelText: () -> Unit,
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
    /** Selecting text: long press to start, handles to adjust, tap to dismiss. */
    onSelectWord: (pageIndex: Int, point: Offset) -> Unit,
    onMoveSelectionHandle: (isStart: Boolean, point: Offset) -> Unit,
    onClearSelection: () -> Unit,
    /** What to do with the selected text. */
    onCopySelection: () -> Unit,
    onHighlightSelection: () -> Unit,
    /** A box was dragged around part of the reader; capture what it framed. */
    onCaptureViewport: (
        tiles: List<CaptureTile>,
        area: Rect,
        background: Long,
        originPage: Int,
        mask: List<Offset>,
    ) -> Unit,
    onClearPage: (Int) -> Unit,
    onClearAll: () -> Unit,
    /** Long-pressing the capture tool chose a shape. */
    onCaptureLasso: (Boolean) -> Unit,
    /** The reader has taken the scroll a history step asked for. */
    onJumpHandled: () -> Unit,
    onShowMetadata: (Boolean) -> Unit,
    // ---------------------------------------------------------- page organiser --
    onShowPageOrganiser: (Boolean) -> Unit,
    /** The reader asked for a sheet; put the question up. */
    onShowBlankPage: () -> Unit,
    /** They answered it: add a sheet of this size and colour. */
    onAddBlankPage: (at: Int, sheet: BlankSheet) -> Unit,
    onDismissBlankPage: () -> Unit,
    /** Remove the page in view. Undoable, like every other page edit. */
    onDeleteCurrentPage: () -> Unit,
    /**
     * One page-tree change. A single sealed type rather than a callback each,
     * because every new operation would otherwise add another parameter to an
     * already long list — and to keep the set readable beside the engine's own
     * `Command` enum.
     */
    onPageAction: (PageAction) -> Unit,
    /** Write the page changes back over the file that was opened. */
    onSaveDocument: () -> Unit,
    /** Write the page changes to a file the user picks. */
    onSaveCopy: () -> Unit,
    /** Open the grid for choosing pages to write out as their own PDF. */
    onExportPages: () -> Unit,
    /** Pick a file to bring pages in from. */
    onImportPages: () -> Unit,
    /** The pages chosen to export. */
    onPagesChosenToExport: (List<Int>) -> Unit,
    onCancelExport: () -> Unit,
    /** The pages chosen out of the file being imported from. */
    onPagesChosenToImport: (List<Int>) -> Unit,
    onCancelImport: () -> Unit,
    /** Page sizes and thumbnails of the file being imported from. */
    importSourcePageSize: suspend (Int) -> PageSize?,
    importSourceRenderer: suspend (Int, Float) -> android.graphics.Bitmap?,
    /** The snackbar has shown `state.message`; clear it. */
    onMessageShown: () -> Unit,
    // ----------------------------------------------------------------- capture --
    /** Re-render the capture on screen at another resolution or in another format. */
    onCaptureScale: (CaptureScale) -> Unit,
    onCaptureFormat: (CaptureFormat) -> Unit,
    /** What fills the capture where no page reaches. */
    onCaptureFill: (CaptureFill) -> Unit,
    onSaveCapture: () -> Unit,
    onShareCapture: () -> Unit,
    onCopyCapture: () -> Unit,
    onDismissCapture: () -> Unit,
    /** The share sheet has been raised; the capture can be let go of. */
    onCaptureShared: () -> Unit,
    // Markup on the capture. The shapes are Kotlin's until they are committed,
    // and the engine's from then on — see roadmap decision 4.7.
    onMarkupTool: (MarkupTool) -> Unit,
    /** Put the markup tool down, so a stray finger cannot draw on the picture. */
    onDisarmMarkup: () -> Unit,
    onMarkupColor: (Long) -> Unit,
    /** Nib width, or the highlighter's intensity — see `MarkupTool.isIntensity`. */
    onMarkupSize: (MarkupTool, Float) -> Unit,
    /** Solid, dashed or dash-dot, for the line tool. */
    onMarkupStyle: (MarkupStyle) -> Unit,
    onCommitMarkup: (MarkupShape) -> Unit,
    /** A stroke that was held still before lifting; ask what shape it is. */
    onRecogniseMarkup: (List<Offset>) -> Unit,
    onUndoMarkup: () -> Unit,
    /** Words on a capture were dragged to a new place. */
    onMoveMarkup: (index: Int, delta: Offset) -> Unit,
    /** A caption on a capture was tapped; the ribbon now edits it. */
    onSelectMarkup: (index: Int) -> Unit,
    /** Two fingers with a caption in hand on a capture: that big. */
    onScaleMarkup: (factor: Float) -> Unit,
    /** A caption on a capture was rewritten. */
    onRewriteMarkup: (index: Int, text: String) -> Unit,
    /** Its words were cleared, which is how one is deleted. */
    onEraseMarkup: (index: Int) -> Unit,
    onSubmitPassword: (String) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    // How much room the bar has, asked of the window rather than assumed from a
    // device class: a tablet in portrait, a phone in landscape and a freeform
    // window are the same question, and the width answers it for all three.
    val compactWidth = LocalConfiguration.current.screenWidthDp < COMPACT_WIDTH_DP

    val snackbarHostState = remember { SnackbarHostState() }

    // Told once, when the width first says so. The rail costs a quarter of a
    // phone screen, and the page is what someone opened the app for.
    LaunchedEffect(compactWidth) { if (compactWidth) onNarrowScreen() }

    // Keyed on the message itself, so two different messages in a row both show
    // and the effect does not restart on every unrelated recomposition.
    //
    // Stands down while the page organiser is open. A modal sheet draws over the
    // `Scaffold` and its snackbar host with it, so showing one there would consume
    // the message behind the sheet and leave the user with no sign that anything
    // happened — the organiser displays it inline instead.
    LaunchedEffect(state.message, state.showPageOrganiser) {
        if (state.showPageOrganiser) return@LaunchedEffect
        val message = state.message ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        onMessageShown()
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = state.documentName.ifBlank { "Pagify" },
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            // Cut, never wrapped. Given a narrow slot the name
                            // broke between letters and ran down the screen —
                            // "HS / Pag / e 10 / of / 149" is not a title.
                            softWrap = false,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (state.isReady) {
                            Text(
                                text = "Page ${state.currentPageLabel} of ${state.pageCount}",
                                style = MaterialTheme.typography.labelSmall,
                                maxLines = 1,
                                softWrap = false,
                                overflow = TextOverflow.Ellipsis,
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
                        // Saving lived only inside the page-organiser sheet, which
                        // is where you go to reorder and delete pages — not
                        // anywhere a reader who has just highlighted something
                        // would look. The work was saveable all along and appeared
                        // not to be, which comes to the same thing.
                        SaveAction(
                            hasUnsavedWork = state.editState.dirty ||
                                state.unsavedMarkCount > 0,
                            isSaving = state.isSaving,
                            onSave = onSaveDocument,
                            onSaveCopy = onSaveCopy,
                        )
                        // Everything past this point folds into an overflow when
                        // the bar is too narrow to hold it. Undo, redo and save
                        // stay put at any width: they are what an edit needs.
                        ReaderActionBar(
                            inlineLimit = if (compactWidth) 0 else WIDE_INLINE_ACTIONS,
                            actions = listOf(
                                ReaderAction(
                                    icon = if (state.isRecording) {
                                        Icons.Filled.StopCircle
                                    } else {
                                        Icons.Filled.FiberManualRecord
                                    },
                                    label = if (state.isRecording) {
                                        "Stop recording and save the render timeline"
                                    } else {
                                        "Record a render timeline"
                                    },
                                    tint = if (state.isRecording) {
                                        MaterialTheme.colorScheme.error
                                    } else {
                                        MaterialTheme.colorScheme.onSurfaceVariant
                                    },
                                    onClick = onToggleRecording,
                                ),
                                ReaderAction(
                                    icon = Icons.AutoMirrored.Filled.ViewSidebar,
                                    label = if (state.showThumbnails) {
                                        "Hide page thumbnails"
                                    } else {
                                        "Show page thumbnails"
                                    },
                                    tint = if (state.showThumbnails) {
                                        MaterialTheme.colorScheme.primary
                                    } else {
                                        MaterialTheme.colorScheme.onSurfaceVariant
                                    },
                                    onClick = onToggleThumbnails,
                                ),
                                ReaderAction(
                                    icon = Icons.AutoMirrored.Filled.RotateRight,
                                    label = "Rotate",
                                    onClick = onRotate,
                                ),
                                ReaderAction(
                                    icon = Icons.Filled.GridView,
                                    // The tint is the only thing distinguishing a
                                    // document with unsaved page changes from one
                                    // without, so it belongs in the label too.
                                    label = if (state.editState.dirty) {
                                        "Organise pages — unsaved changes"
                                    } else {
                                        "Organise pages"
                                    },
                                    tint = if (state.editState.dirty) {
                                        MaterialTheme.colorScheme.primary
                                    } else {
                                        MaterialTheme.colorScheme.onSurfaceVariant
                                    },
                                    onClick = { onShowPageOrganiser(true) },
                                ),
                                ReaderAction(
                                    icon = Icons.Filled.AddPhotoAlternate,
                                    label = "Add a blank page",
                                    onClick = onShowBlankPage,
                                ),
                                ReaderAction(
                                    icon = Icons.Filled.FileUpload,
                                    label = "Export pages…",
                                    onClick = onExportPages,
                                ),
                                ReaderAction(
                                    icon = Icons.Filled.FileDownload,
                                    label = "Import pages…",
                                    onClick = onImportPages,
                                ),
                                ReaderAction(
                                    // Acts on the page in view. Deleting the page
                                    // you are looking at meant opening the grid and
                                    // finding it again, which is a lot of asking
                                    // for something you are already looking at.
                                    icon = Icons.Filled.Delete,
                                    label = "Delete this page",
                                    onClick = { onDeleteCurrentPage() },
                                ),
                                ReaderAction(
                                    icon = Icons.Filled.Info,
                                    label = "Document details",
                                    onClick = { onShowMetadata(true) },
                                ),
                                ReaderAction(
                                    icon = Icons.Filled.FolderOpen,
                                    label = "Open a PDF",
                                    onClick = onPickDocument,
                                ),
                            ),
                        )
                    } else {
                        // Nothing open yet, so the only thing worth offering is
                        // the way to open something.
                        IconButton(onClick = onPickDocument) {
                            Icon(Icons.Filled.FolderOpen, contentDescription = "Open a PDF")
                        }
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
                    onRequestNote = onRequestNote,
                    onPlaceText = onPlaceText,
                    onMoveText = onMoveText,
                    onSelectText = onSelectText,
                    onScaleText = onScaleText,
                    onTurnZoomedPage = onTurnZoomedPage,
                    onEditText = onEditText,
                    onPageMarksNeeded = onPageMarksNeeded,
                    showViewfinder = showViewfinder,
                    viewfinderMinimized = viewfinderMinimized,
                    onViewfinderMinimized = onViewfinderMinimized,
                    viewfinderHandle = viewfinderHandle,
                    onViewfinderHandleMoved = onViewfinderHandleMoved,
                    onOpenNote = onOpenNote,
                    onEraseStart = onEraseStart,
                    onErase = onErase,
                    onEraseEnd = onEraseEnd,
                    onHighlightMissed = onHighlightMissed,
                    onSelectWord = onSelectWord,
                    onMoveSelectionHandle = onMoveSelectionHandle,
                    onClearSelection = onClearSelection,
                    onCaptureViewport = onCaptureViewport,
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
                val highlighterOnScan = state.tool == AnnotationTool.Highlight &&
                    state.currentPage in state.pagesWithoutSelectableText
                var hintDismissed by remember(state.currentPage) { mutableStateOf(false) }

                // Recorded only while the highlighter is the live tool, which is
                // the only time the answer is interesting — and it is the line
                // that separates "the page has text we failed to find" from "the
                // hint is showing and the reader ignored it".
                val highlighterLive = state.tool == AnnotationTool.Highlight
                LaunchedEffect(highlighterLive, highlighterOnScan, state.currentPage, hintDismissed) {
                    if (!highlighterLive) return@LaunchedEffect
                    SessionRecorder.record(
                        kind = "NO_TEXT_HINT",
                        detail = "show=$highlighterOnScan dismissed=$hintDismissed " +
                            "page=${state.currentPage} " +
                            "pagesWithoutText=${state.pagesWithoutSelectableText.size}",
                    )
                }

                // Recognition takes seconds on a heavy page, and it starts on the
                // first touch of a highlight drag. Without something on screen the
                // gesture looks like it did nothing at all, which is precisely the
                // complaint that led here.
                if (state.tool == AnnotationTool.Snapshot && state.capture == null) {
                    CaptureHint(
                        lasso = state.captureLasso,
                        modifier = Modifier
                            .align(Alignment.BottomCenter)
                            .padding(bottom = 94.dp),
                    )
                } else if (highlighterLive && state.currentPage in state.pagesBeingRecognised) {
                    RecognisingTextHint(
                        modifier = Modifier
                            .align(Alignment.BottomCenter)
                            .padding(bottom = 94.dp),
                    )
                } else if (highlighterOnScan && !hintDismissed) {
                    NoTextOnPageHint(
                        onUseMarker = {
                            onSelectTool(AnnotationTool.Pen)
                            hintDismissed = true
                        },
                        onDismiss = { hintDismissed = true },
                        modifier = Modifier
                            .align(Alignment.BottomCenter)
                            .padding(bottom = 94.dp),
                    )
                }

                // The bar for a selection, above the tool ribbon. Only while text
                // is selected, and it goes as soon as the selection does.
                state.selection?.let { selection ->
                    TextSelectionBar(
                        characters = selection.text.length,
                        onCopy = onCopySelection,
                        onHighlight = onHighlightSelection,
                        onDismiss = onClearSelection,
                        modifier = Modifier
                            .align(Alignment.BottomCenter)
                            .padding(bottom = 94.dp),
                    )
                }

                AnnotationToolbar(
                    selectedTool = state.tool,

                    penColor = state.penColor,
                    onSelectTool = onSelectTool,
                    strokeWidth = state.annotationStrokeWidth,
                    lineStyle = state.annotationStyle,
                    textFont = state.textFont,
                    textSizePoints = state.textSizePoints,
                    textCurveDegrees = state.textCurveDegrees,
                    textSizeCeiling = state.textSizeCeiling,
                    textBendApplies = state.textBendApplies,
                    onTextFont = onTextFont,
                    onTextSize = onTextSize,
                    onTextCurve = onTextCurve,
                    onStrokeWidth = onStrokeWidth,
                    onLineStyle = onLineStyle,
                    onPenColorChange = onPenColorChange,
                    marksOnPage = state.annotationsOnPage,
                    marksInDocument = state.annotationsInDocument,
                    onClearPage = { onClearPage(state.currentPage) },
                    onClearAll = onClearAll,
                    captureLasso = state.captureLasso,
                    onCaptureLasso = onCaptureLasso,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 20.dp),
                )
            }
        }

        // Full screen, above everything: the picture is a workspace rather than a
        // menu, and a sheet gave it a third of the display while the controls took
        // the rest.
        state.capture?.let { capture ->
            CaptureEditor(
                preview = capture,
                isCapturing = state.isCapturing,
                markup = state.markup,
                markupTool = state.markupTool,
                markupArmed = state.markupArmed,
                onDisarmMarkup = onDisarmMarkup,
                markupColor = state.markupColor,
                markupSize = state.markupSizes[state.markupTool] ?: state.markupTool.defaultSize,
                markupStyle = state.markupStyle,
                onMarkupStyle = onMarkupStyle,
                onScaleChange = onCaptureScale,
                onFormatChange = onCaptureFormat,
                onMarkupTool = onMarkupTool,
                onMarkupColor = onMarkupColor,
                onMarkupSize = onMarkupSize,
                onCommitMarkup = onCommitMarkup,
                onRecogniseMarkup = onRecogniseMarkup,
                onUndoMarkup = onUndoMarkup,
                onMoveMarkup = onMoveMarkup,
                onSelectMarkup = onSelectMarkup,
                onScaleMarkup = onScaleMarkup,
                onRewriteMarkup = onRewriteMarkup,
                onEraseMarkup = onEraseMarkup,
                selectedMarkup = state.selectedMarkupIndex,
                textFont = state.textFont,
                textSizePoints = state.textSizePoints,
                textCurveDegrees = state.textCurveDegrees,
                onTextFont = onTextFont,
                onTextSize = onTextSize,
                onTextCurve = onTextCurve,
                fill = state.captureFill,
                onFillChange = onCaptureFill,
                onSaveToGallery = onSaveCapture,
                onShare = onShareCapture,
                onCopy = onCopyCapture,
                onDismiss = onDismissCapture,
            )
        }

        // Launched from an effect rather than straight from the button, because
        // the file has to be written first: a share sheet raised before the bytes
        // are on disk hands the receiving app an empty file.
        val shareContext = LocalContext.current
        LaunchedEffect(state.captureToShare) {
            val share = state.captureToShare ?: return@LaunchedEffect
            val format = CaptureFormat.entries.first { it.mimeType == share.mimeType }
            shareContext.startActivity(CaptureExport.shareIntent(share.uri, format))
            onCaptureShared()
        }

        state.openNote?.let { note ->
            NoteReader(
                text = note.text,
                onDelete = onDeleteNote,
                onDismiss = onCloseNote,
            )
        }

        state.pendingNote?.let {
            NoteComposer(onConfirm = onConfirmNote, onDismiss = onCancelNote)
        }

        state.textBeingWritten?.let { pending ->
            TextComposer(
                // What the tool is, not what the path looks like: the bend is a
                // setting now, so a curved caption starts life as a single tap
                // like any other and the path cannot say which tool made it.
                curved = pending.bends,
                initial = pending.initial,
                editing = pending.editing != null,
                onConfirm = onCommitText,
                onDismiss = onCancelText,
            )
        }

        if (state.showMetadataSheet && state.metadata != null) {
            ModalBottomSheet(onDismissRequest = { onShowMetadata(false) }) {
                MetadataSheet(state.metadata)
            }
        }

        // Choosing pages out of *this* document, to write somewhere else.
        if (state.choosingPagesToExport) {
            ModalBottomSheet(
                onDismissRequest = onCancelExport,
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                PagePicker(
                    pageCount = state.pageCount,
                    confirmLabel = { count ->
                        if (count == 1) "Export 1 page" else "Export $count pages"
                    },
                    onConfirm = onPagesChosenToExport,
                    onCancel = onCancelExport,
                    pageSizeProvider = pageSizeProvider,
                    renderer = thumbnailRenderer,
                )
            }
        }

        // And choosing pages out of a file just opened, to bring in here.
        state.importSource?.let { source ->
            ModalBottomSheet(
                onDismissRequest = onCancelImport,
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                PagePicker(
                    pageCount = source.pageCount,
                    confirmLabel = { count ->
                        if (count == 1) "Import 1 page" else "Import $count pages"
                    },
                    onConfirm = onPagesChosenToImport,
                    onCancel = onCancelImport,
                    pageSizeProvider = importSourcePageSize,
                    renderer = importSourceRenderer,
                )
            }
        }

        state.blankPageAfter?.let { at ->
            BlankPageSheet(
                template = state.pageSizes[state.currentPage],
                onAdd = { sheet -> onAddBlankPage(at, sheet) },
                onDismiss = onDismissBlankPage,
            )
        }

        if (state.showPageOrganiser && state.isReady) {
            ModalBottomSheet(
                onDismissRequest = { onShowPageOrganiser(false) },
                // Straight to full height, skipping the half-open state a sheet
                // normally rests at. This one is a working panel, not a short menu:
                // at half height its page grid ran past the bottom of the screen and
                // took Save and Close with it, and nothing on screen suggested the
                // sheet could be dragged up to reach them.
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                PageOrganiser(
                    pageCount = state.pageCount,
                    currentPage = state.currentPage,
                    editState = state.editState,
                    unsavedMarks = state.unsavedMarkCount,
                    isSaving = state.isSaving,
                    onAction = onPageAction,
                    onSave = onSaveDocument,
                    onSaveCopy = onSaveCopy,
                    onClose = { onShowPageOrganiser(false) },
                    pageContentRevision = state.pageContentRevision,
                    onExportPages = onExportPages,
                    onImportPages = onImportPages,
                    pageSizeProvider = pageSizeProvider,
                    // The rail's throttled renderer, not the full-size one: this
                    // grid asks for every page at once, and a dozen full renders
                    // would queue ahead of the page being read.
                    renderer = thumbnailRenderer,
                    message = state.message,
                    onMessageShown = onMessageShown,
                )
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
    onRequestNote: (pageIndex: Int, anchor: Offset) -> Unit,
    onPlaceText: (pageIndex: Int, path: List<Offset>) -> Unit,
    /** Placed text was dragged to a new spot on the same page. */
    onMoveText: (id: Long, delta: Offset) -> Unit,
    /** A caption was tapped, so the ribbon's controls now belong to it. */
    onSelectText: (id: Long?) -> Unit,
    /** Two fingers with a caption in hand: that big. */
    onScaleText: (factor: Float) -> Unit,
    /** Swiped past the end of a magnified page; move to the next or previous. */
    onTurnZoomedPage: (delta: Int) -> Boolean,
    /** A caption was double-tapped; rewrite its words. */
    onEditText: (id: Long) -> Unit,
    /** This page is on screen; load any marks the file already holds for it. */
    onPageMarksNeeded: (Int) -> Unit,
    onOpenNote: (Annotation.Note) -> Unit,
    onEraseStart: () -> Unit,
    onErase: (pageIndex: Int, point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    onHighlightMissed: (Int) -> Unit,
    /** Selecting text: long press to start, handles to adjust, tap to dismiss. */
    onSelectWord: (pageIndex: Int, point: Offset) -> Unit,
    onMoveSelectionHandle: (isStart: Boolean, point: Offset) -> Unit,
    onClearSelection: () -> Unit,
    /** A box was dragged around part of the reader; capture what it framed. */
    onCaptureViewport: (
        tiles: List<CaptureTile>,
        area: Rect,
        background: Long,
        originPage: Int,
        mask: List<Offset>,
    ) -> Unit,
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
    /** Whether the viewfinder may appear at all, and whether it is folded away. */
    showViewfinder: Boolean,
    viewfinderMinimized: Boolean,
    onViewfinderMinimized: (Boolean) -> Unit,
    viewfinderHandle: Offset,
    onViewfinderHandleMoved: (Offset) -> Unit,
) {
    val listState = rememberLazyListState()
    val coroutineScope = rememberCoroutineScope()
    val density = LocalDensity.current

    // Where the navigator's viewport indicator comes from while pinned.
    var window by remember { mutableStateOf(ViewportWindow.Full) }

    /**
     * Where each page currently sits, in the reader's own pixels.
     *
     * A capture spans whatever is on screen, so it needs the layout — and the
     * layout is only known once the pages have been placed. Reported by each page
     * rather than derived from `LazyListState`, which knows an item's vertical
     * offset but nothing about where the page inside it was drawn.
     *
     * Stored in **window** coordinates, exactly as reported. Converting to the
     * reader's space at write time looked tidier and was wrong: the origin it
     * would subtract is itself only known after the reader has been positioned, so
     * the first pages to lay out recorded themselves against a stale one and never
     * corrected. The capture converts at read time instead, from the overlay's own
     * origin, which is the same frame the drag was measured in.
     *
     * A plain map, not state: it is written during layout and read only when a
     * capture gesture ends, and making it observable would recompose the reader on
     * every scroll frame for no one's benefit.
     */
    val pageBounds = remember { mutableMapOf<Int, Rect>() }
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
            val wantsText = state.tool == AnnotationTool.Highlight
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
                quarterTurns = state.rotationQuarterTurns,
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
                annotationRevision = state.annotationRevision,
                textSegments = pinnedSegments,
                tool = state.tool,
                captureLasso = state.captureLasso,

                penColor = state.penColor,
                strokeWidth = state.annotationStrokeWidth,
                lineStyle = state.annotationStyle,
                onAddAnnotation = onAddAnnotation,
                onRequestNote = onRequestNote,
                onPlaceText = onPlaceText,
                onMoveText = onMoveText,
                onSelectText = onSelectText,
                selectedText = state.selectedTextId,
                onScaleText = onScaleText,
                onEditText = onEditText,
                onTurnPage = onTurnZoomedPage,
                onPageMarksNeeded = onPageMarksNeeded,
                onOpenNote = onOpenNote,
                onEraseStart = onEraseStart,
                onErase = { point, tolerance -> onErase(pinnedPage, point, tolerance) },
                onEraseEnd = onEraseEnd,
                onCaptureViewport = onCaptureViewport,
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
                SessionRecorder.record("ZOOM_ENTER", "target=$targetZoom at=$position")
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
                                // Turned, like the row it is measuring: this offset
                                // is how far down the list the page starts, and a
                                // rotated page is a different height.
                                val size = state.displaySize(page)
                                    ?: pageSizeProvider(page)?.turned(state.rotationQuarterTurns)

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
                            // A caption in hand takes the pinch. Not a race with
                            // the page's zoom but a decision made before the
                            // fingers land: while one is held, two fingers mean
                            // "this big", and the page holds still. Putting it
                            // down — a tap on empty page — gives the zoom back.
                            if (state.selectedTextId != null) {
                                onScaleText(factor)
                                return@pinchToZoom
                            }
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
                        // Watched on the Initial pass, not detected on the Main
                        // one: every page carries its own tap handler, a child is
                        // served first, and `detectTapGestures` consumes what it
                        // handles — so the zoom never saw a tap at all. See
                        // `doubleTapToZoom`.
                        .doubleTapToZoom { position ->
                            // Nothing while a caption is in hand: a double tap on
                            // one opens it for rewriting, and the page must not
                            // jump to twice its size at the same time.
                            if (state.selectedTextId != null) return@doubleTapToZoom
                            SessionRecorder.record("ZOOM_DTAP_LIST", "at=$position")
                            onZoomActivity()
                            // A double-tap is a request for a specific
                            // magnification, so the jump here is the point.
                            enterZoom(position, DOUBLE_TAP_ZOOM)
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

                    // Read here, where the theme is in scope, rather than in the
                    // engine: what shows between two pages is a reader decision.
                    val readerBackground = MaterialTheme.colorScheme.surfaceVariant
                        .toArgb()
                        .toLong() and 0xFFFFFFFFL

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
                                // Nothing while a caption is in hand: the two
                                // fingers resizing it must not also scroll the
                                // document out from under it.
                                if (state.selectedTextId != null) return@twoFingerPan
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
                                onRequestNote = onRequestNote,
                                onPlaceText = onPlaceText,
                                onMoveText = onMoveText,
                                onSelectText = onSelectText,
                                onEditText = onEditText,
                                onPageMarksNeeded = onPageMarksNeeded,
                                onOpenNote = onOpenNote,
                                onEraseStart = onEraseStart,
                                onErase = onErase,
                                onEraseEnd = onEraseEnd,
                                onHighlightMissed = onHighlightMissed,
                                onSelectWord = onSelectWord,
                                onMoveSelectionHandle = onMoveSelectionHandle,
                                onClearSelection = onClearSelection,
                                onBounds = { index, bounds -> pageBounds[index] = bounds },
                                pageSizeProvider = pageSizeProvider,
                                renderer = renderer,
                            )
                        }
                    }

                    // Above the list, so the drag reaches this before the scroll
                    // container can claim it — and so a box can cross a page join,
                    // which is the whole point of capturing what is on screen
                    // rather than what fits on one page.
                    if (state.tool == AnnotationTool.Snapshot) {
                        // The overlay's own position, read at gesture time. The drag
                        // arrives in this element's coordinates and the pages report
                        // themselves in the window's, so one of the two has to move —
                        // and doing it here, from a value that is current, is what
                        // the write-time version could not guarantee.
                        var overlayOrigin by remember { mutableStateOf(Offset.Zero) }

                        Box(
                            Modifier
                                .fillMaxSize()
                                .onGloballyPositioned {
                                    overlayOrigin = it.boundsInWindow().topLeft
                                }
                                .captureOverlay(lasso = state.captureLasso) { local, ring ->
                                    val box = local.translate(overlayOrigin.x, overlayOrigin.y)
                                    val pages = pageBounds.entries
                                        .sortedBy { it.key }
                                        .mapNotNull { (index, bounds) ->
                                            state.pageSizes[index]?.let {
                                                PlacedPage(index, bounds, it)
                                            }
                                        }
                                    val tiles = captureTilesFor(box, pages)
                                    SessionRecorder.record(
                                        kind = "CAPTURE_BOX",
                                        detail = "box=${box.left.toInt()},${box.top.toInt()}.." +
                                            "${box.right.toInt()},${box.bottom.toInt()} " +
                                            "origin=${overlayOrigin.x.toInt()},${overlayOrigin.y.toInt()} " +
                                            "known=${pageBounds.size} sized=${pages.size} " +
                                            "tiles=${tiles.size} " +
                                            pages.take(4).joinToString(" ") {
                                                "p${it.pageIndex}=${it.bounds.top.toInt()}.." +
                                                    "${it.bounds.bottom.toInt()}"
                                            },
                                    )
                                    onCaptureViewport(
                                        tiles,
                                        box,
                                        // The reader's own backdrop, so the gap
                                        // between two pages looks in the picture
                                        // like it looked on screen.
                                        readerBackground,
                                        tiles.firstOrNull()?.pageIndex ?: state.currentPage,
                                        // The ring is in the overlay's own
                                        // pixels like the box was, so it moves
                                        // to window space the same way before
                                        // being made relative to the picture.
                                        captureMaskFor(
                                            box,
                                            ring.map { it + overlayOrigin },
                                        ),
                                    )
                                },
                        )
                    }
                }
            }
        }

        // ------------------------------------------------------------ navigator --
        if (pinnedPage != null && !window.coversEverything && showViewfinder) {
            PageNavigator(
                pageIndex = pinnedPage,
                pageSize = state.pageSizes[pinnedPage],
                window = window,
                thumbnailRenderer = thumbnailRenderer,
                // Folded, it can be anywhere in the reader, so it takes the
                // whole area and places itself; open, it keeps its corner.
                modifier = if (viewfinderMinimized) {
                    Modifier.matchParentSize()
                } else {
                    Modifier.align(Alignment.CenterEnd)
                },
                onRecenter = { fx, fy -> recenterRequest = Offset(fx, fy) },
                minimized = viewfinderMinimized,
                onMinimized = onViewfinderMinimized,
                handlePosition = viewfinderHandle,
                onHandleMoved = onViewfinderHandleMoved,
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
    onRequestNote: (pageIndex: Int, anchor: Offset) -> Unit,
    onPlaceText: (pageIndex: Int, path: List<Offset>) -> Unit,
    /** Placed text was dragged to a new spot on the same page. */
    onMoveText: (id: Long, delta: Offset) -> Unit,
    /** A caption was tapped, so the ribbon's controls now belong to it. */
    onSelectText: (id: Long?) -> Unit,
    /** A caption was double-tapped; rewrite its words. */
    onEditText: (id: Long) -> Unit,
    /** This page is on screen; load any marks the file already holds for it. */
    onPageMarksNeeded: (Int) -> Unit,
    onOpenNote: (Annotation.Note) -> Unit,
    onEraseStart: () -> Unit,
    onErase: (pageIndex: Int, point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    onHighlightMissed: (Int) -> Unit,
    /** A long press on this page; select the word under it. */
    onSelectWord: (pageIndex: Int, point: Offset) -> Unit,
    onMoveSelectionHandle: (isStart: Boolean, point: Offset) -> Unit,
    onClearSelection: () -> Unit,
    /**
     * Where this page ended up, in window coordinates.
     *
     * Reported rather than derived: a capture spans whatever is on screen, and
     * only the layout knows where each page actually landed.
     */
    onBounds: (pageIndex: Int, bounds: Rect) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> android.graphics.Bitmap?,
) {
    val density = LocalDensity.current
    // The page's own size for the geometry, and its laid-out size for the box:
    // a turned page is drawn across its height, so the scale has to come from
    // the turned width or the raster would not fill what it is drawn into.
    val pageSize = state.pageSizes[pageIndex]
    val laidOut = state.displaySize(pageIndex)
    val turns = state.rotationQuarterTurns

    // Pixels per page point, at the width this page is actually drawn.
    val mapping = remember(pageSize, laidOut, turns, pageWidth, density) {
        val size = laidOut ?: return@remember PageMapping.Unmeasured
        if (size.widthPoints <= 0f) return@remember PageMapping.Unmeasured
        PageMapping(
            scale = with(density) { pageWidth.toPx() } / size.widthPoints,
            quarterTurns = turns,
            pageWidthPoints = pageSize?.widthPoints ?: 0f,
            pageHeightPoints = pageSize?.heightPoints ?: 0f,
        )
    }

    // What the marks on this page are about to be drawn through.
    //
    // Recorded because a mark in the wrong place is either the mark or the
    // mapping, and a screenshot cannot tell those apart: ink that failed to turn
    // with its page looks exactly like ink that turned the wrong way.
    LaunchedEffect(mapping, pageIndex) {
        SessionRecorder.record(
            kind = "PAGE_MAPPING",
            detail = "page=$pageIndex turns=${mapping.quarterTurns} " +
                "scale=${mapping.scale} " +
                "pts=${mapping.pageWidthPoints}x${mapping.pageHeightPoints}",
        )
    }

    // Only loaded when the highlighter is actually in use: walking every text run
    // on a page is real work, and no other tool needs it.
    var segments by remember(pageIndex) { mutableStateOf<List<TextSegment>>(emptyList()) }
    val wantsText = state.tool == AnnotationTool.Highlight
    LaunchedEffect(pageIndex, wantsText) {
        if (wantsText && segments.isEmpty()) segments = textSegmentsForPage(pageIndex)
    }

    // Re-read through the revision counter: the store is mutable and identity
    // stable, so Compose cannot otherwise see that a mark was added.
    // Marks already in the file are read the first time a page is drawn, so they
    // can be erased rather than only rendered.
    LaunchedEffect(pageIndex, state.documentRevision) { onPageMarksNeeded(pageIndex) }

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
        knownSize = laidOut,
        pageSizeProvider = { index -> pageSizeProvider(index)?.turned(turns) },
        renderer = renderer,
        // Read straight off the state this composable already has, rather than
        // threaded down from the screen: it changes for exactly the same reason
        // the marks do, and three more parameters to carry one integer would be
        // three more places to forget it.
        contentRevision = state.pageContentRevision,
        modifier = Modifier
            .onGloballyPositioned { onBounds(pageIndex, it.boundsInWindow()) }
            .annotationLayer(
            pageIndex = pageIndex,
            annotations = annotations,
            revision = state.annotationRevision,
            textSegments = segments,
            tool = state.tool,

            penColor = state.penColor,
            strokeWidth = state.annotationStrokeWidth,
            lineStyle = state.annotationStyle,
            mapping = mapping,
            onAdd = onAddAnnotation,
            onRequestNote = { anchor -> onRequestNote(pageIndex, anchor) },
            onPlaceText = { path -> onPlaceText(pageIndex, path) },
            onMoveText = onMoveText,
            onSelectText = onSelectText,
            selectedText = state.selectedTextId,
            onEditText = onEditText,
            onOpenNote = onOpenNote,
            onEraseStart = onEraseStart,
            onErase = { point, tolerance -> onErase(pageIndex, point, tolerance) },
            onEraseEnd = onEraseEnd,
            onHighlightMissed = { onHighlightMissed(pageIndex) },
            // Only the page that holds it: two pages drawing the same selection
            // would put a handle on each.
            selection = state.selection?.takeIf { it.pageIndex == pageIndex },
            onSelectWord = { at -> onSelectWord(pageIndex, at) },
            onMoveSelectionHandle = onMoveSelectionHandle,
            onClearSelection = onClearSelection,
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


/**
 * Below this width the top bar folds its extras into an overflow.
 *
 * 600dp is the platform's own compact/medium boundary, and it lands where it
 * needs to: a phone in portrait is around 360dp and cannot hold nine icon
 * buttons, while a tablet in portrait is 800 and holds them comfortably.
 */
private const val COMPACT_WIDTH_DP = 600

/**
 * How many actions a wide window shows in the bar before the rest fold away.
 *
 * Four, because undo, redo, save and save-as already sit beside them and stay
 * put at any width — so the bar carries eight buttons at its widest, and what
 * is left is enough for a document name to be readable rather than merely
 * present.
 */
private const val WIDE_INLINE_ACTIONS = 4
