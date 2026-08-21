package com.hsilighting.pagify

import android.Manifest.permission.WRITE_EXTERNAL_STORAGE
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.geometry.Offset
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.hsilighting.pagify.core.BlankFrameDetector
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.isDark
import com.hsilighting.pagify.ui.components.PageAction
import com.hsilighting.pagify.ui.PagifyApp
import com.hsilighting.pagify.ui.reader.PdfReaderScreen
import com.hsilighting.pagify.ui.reader.PdfReaderViewModel
import com.hsilighting.pagify.ui.theme.PagifyTheme
import kotlinx.coroutines.flow.MutableStateFlow

class MainActivity : ComponentActivity() {

    /**
     * A PDF handed to us by another app (a mail attachment, a browser download).
     *
     * Held in a flow rather than read once in `onCreate` so that `onNewIntent` —
     * which fires when the activity is already running — reaches the same handler.
     */
    private val incomingDocument = MutableStateFlow<Uri?>(null)

    /** Reads real screen pixels during zoom gestures. See [BlankFrameDetector]. */
    private lateinit var blankFrameDetector: BlankFrameDetector

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        blankFrameDetector = BlankFrameDetector(window)

        incomingDocument.value = viewableUri(intent)

        setContent {
            // The view model is made *above* the theme, not inside it: the
            // chosen theme lives in the view model, and a theme that wraps the
            // thing holding its own setting cannot read it.
            val viewModel: PdfReaderViewModel = viewModel()
            val settings by viewModel.settings.collectAsStateWithLifecycle()

            PagifyTheme(darkTheme = settings.theme.isDark(isSystemInDarkTheme())) {
                val state by viewModel.state.collectAsStateWithLifecycle()
                val incoming by incomingDocument.collectAsStateWithLifecycle()
                val recents by viewModel.recents.collectAsStateWithLifecycle()

                LaunchedEffect(incoming) {
                    incoming?.let { uri ->
                        viewModel.open(uri)
                        // Cleared so a configuration change does not reopen it and
                        // discard the page the user had scrolled to.
                        incomingDocument.value = null
                    }
                }

                val picker = rememberLauncherForActivityResult(
                    OpenWritableDocument(),
                ) { uri ->
                    if (uri != null) {
                        // Without this the grant expires with the process, so a
                        // document reopened after a low-memory kill would fail.
                        // Write is taken alongside read because saving edits back
                        // to the file the user opened needs it, and it cannot be
                        // asked for later — the grant is fixed when the picker
                        // returns.
                        runCatching {
                            contentResolver.takePersistableUriPermission(
                                uri,
                                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                            )
                        }.onFailure {
                            // A read-only provider refuses the write half. The
                            // document still opens, and "Save a copy" is the way
                            // out; failing the open over it would be worse.
                            runCatching {
                                contentResolver.takePersistableUriPermission(
                                    uri,
                                    Intent.FLAG_GRANT_READ_URI_PERMISSION,
                                )
                            }
                        }
                        viewModel.open(uri)
                    }
                }

                /**
                 * Where "Save a copy" writes.
                 *
                 * `CreateDocument` rather than a path of our own choosing: the user
                 * picks the destination, which is both the only way to write outside
                 * the sandbox and what makes the file findable again afterwards.
                 */
                val copyPicker = rememberLauncherForActivityResult(
                    ActivityResultContracts.CreateDocument(PDF_MIME_TYPE),
                ) { uri -> uri?.let(viewModel::saveCopyTo) }

                val openPicker = remember { { picker.launch(arrayOf(PDF_MIME_TYPE)) } }

                /**
                 * Saving a capture to the gallery, below API 29 only.
                 *
                 * From API 29 on, MediaStore's scoped storage needs no permission
                 * at all, so this is never reached there — asking anyway would put
                 * a permission dialog in front of an action that does not need one.
                 */
                val storagePermission = rememberLauncherForActivityResult(
                    ActivityResultContracts.RequestPermission(),
                ) { granted ->
                    if (granted) viewModel.saveCaptureToGallery()
                    else viewModel.noteCaptureNeedsStorage()
                }

                val recordingToast: () -> Unit = {
                    // App-private external storage, so the file can be pulled
                    // with adb without any permission prompt.
                    viewModel.toggleRecording(getExternalFilesDir(null))?.let { message ->
                        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
                    }
                }

                PagifyApp(
                    state = state,
                    recents = recents,
                    onOpenRecent = { viewModel.open(it.uri.toUri()) },
                    onForgetRecent = { viewModel.forgetDocument(it.uri) },
                    onPickDocument = openPicker,
                    onClearLibrary = viewModel::clearLibrary,
                    onShowThumbnails = viewModel::setThumbnails,
                    settings = settings,
                    onThemeChange = viewModel::setTheme,
                    onShowViewfinder = viewModel::setShowViewfinder,
                    onToggleRecording = recordingToast,
                    onReturnToLibrary = viewModel::returnToLibrary,
                ) {
                    PdfReaderScreen(
                        state = state,
                        onPickDocument = openPicker,
                        onPageVisible = viewModel::onPageVisible,
                        onZoomInOn = viewModel::zoomInOn,
                        onZoomTo = viewModel::zoomTo,
                        onZoomActivity = { blankFrameDetector.onZoomActivity() },
                        onContentBounds = blankFrameDetector::setContentBounds,
                        peekRenderedPage = viewModel::peekRenderedPage,
                        annotationsForPage = viewModel.annotations::forPage,
                        textSegmentsForPage = viewModel::textSegments,
                        onAddAnnotation = viewModel::addAnnotation,
                        onRequestNote = viewModel::requestNote,
                        onPageMarksNeeded = viewModel::loadSavedMarks,
                        onConfirmNote = viewModel::confirmNote,
                        onCancelNote = viewModel::cancelNote,
                        onOpenNote = viewModel::openNote,
                        onCloseNote = viewModel::closeNote,
                        onDeleteNote = viewModel::deleteOpenNote,
                        onSelectTool = viewModel::selectTool,
                        onStrokeWidth = viewModel::setStrokeWidth,
                        onLineStyle = viewModel::setLineStyle,
                        onPenColorChange = viewModel::setPenColor,
                        onUndoAnnotation = viewModel::undoAnnotation,
                        onRedoAnnotation = viewModel::redoAnnotation,
                        onEraseStart = viewModel::beginErase,
                        onErase = viewModel::eraseAt,
                        onEraseEnd = viewModel::endErase,
                        onClearPage = viewModel::clearPage,
                        onClearAll = viewModel::clearAllAnnotations,
                        onHighlightMissed = viewModel::noteHighlightFoundNothing,
                        onSelectWord = viewModel::selectWordAt,
                        onMoveSelectionHandle = viewModel::moveSelectionHandle,
                        onClearSelection = viewModel::clearSelection,
                        onCopySelection = viewModel::copySelection,
                        onHighlightSelection = viewModel::highlightSelection,
                        onCaptureViewport = viewModel::capture,
                        onCaptureLasso = viewModel::setCaptureLasso,
                        onJumpHandled = viewModel::jumpHandled,
                        onViewportWidth = viewModel::onViewportWidthChanged,
                        onRotate = viewModel::rotate,
                        onToggleThumbnails = viewModel::toggleThumbnails,
                        onNarrowScreen = viewModel::onNarrowScreen,
                        showViewfinder = settings.showViewfinder,
                        viewfinderMinimized = settings.viewfinderMinimized,
                        onViewfinderMinimized = viewModel::setViewfinderMinimized,
                        viewfinderHandle = Offset(
                            settings.viewfinderHandleX,
                            settings.viewfinderHandleY,
                        ),
                        onViewfinderHandleMoved = viewModel::setViewfinderHandle,
                        onToggleRecording = recordingToast,
                        onShowMetadata = viewModel::showMetadata,
                        onShowPageOrganiser = viewModel::showPageOrganiser,
                        onPageAction = { action ->
                            when (action) {
                                is PageAction.Delete -> viewModel.deletePage(action.index)
                                is PageAction.InsertBlankAt -> viewModel.insertBlankPage(action.at)
                                is PageAction.Move -> viewModel.movePage(action.from, action.to)
                                is PageAction.Rotate -> viewModel.rotatePage(action.index)
                                PageAction.Undo -> viewModel.undoEdit()
                                PageAction.Redo -> viewModel.redoEdit()
                            }
                        },
                        onSaveDocument = { viewModel.save() },
                        onSaveCopy = { copyPicker.launch(suggestedCopyName(state.documentName)) },
                        onMessageShown = viewModel::messageShown,
                        onCaptureScale = viewModel::setCaptureScale,
                        onCaptureFormat = viewModel::setCaptureFormat,
                        onCaptureFill = viewModel::setCaptureFill,
                        onSaveCapture = {
                            if (CaptureExport.galleryNeedsPermission()) {
                                storagePermission.launch(WRITE_EXTERNAL_STORAGE)
                            } else {
                                viewModel.saveCaptureToGallery()
                            }
                        },
                        onShareCapture = viewModel::shareCapture,
                        onCopyCapture = viewModel::copyCapture,
                        onDismissCapture = viewModel::dismissCapture,
                        onCaptureShared = viewModel::captureShared,
                        onMarkupTool = viewModel::setMarkupTool,
                        onDisarmMarkup = viewModel::disarmMarkup,
                        onMarkupColor = viewModel::setMarkupColor,
                        onMarkupSize = viewModel::setMarkupSize,
                        onMarkupStyle = viewModel::setMarkupStyle,
                        onCommitMarkup = viewModel::addMarkup,
                        onRecogniseMarkup = viewModel::recogniseAndAddMarkup,
                        onUndoMarkup = viewModel::undoMarkup,
                        onSubmitPassword = viewModel::submitPassword,
                        pageSizeProvider = viewModel::pageSize,
                        renderer = viewModel::renderPage,
                        thumbnailRenderer = viewModel::renderThumbnail,
                    )
                }
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        blankFrameDetector.release()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        viewableUri(intent)?.let { incomingDocument.value = it }
    }

    private fun viewableUri(intent: Intent?): Uri? = intent
        ?.takeIf { it.action == Intent.ACTION_VIEW || it.action == Intent.ACTION_SEND }
        ?.data

    companion object {
        const val PDF_MIME_TYPE = "application/pdf"

        /**
         * A name for "Save a copy" that will not collide with the original.
         *
         * The picker lets the user change it, so this only has to be a sensible
         * starting point — and one that makes clear which file it came from.
         */
        fun suggestedCopyName(documentName: String): String {
            val base = documentName.ifBlank { "Document" }.removeSuffix(".pdf")
            return "$base (edited).pdf"
        }
    }
}

/**
 * `ACTION_OPEN_DOCUMENT`, asking for write access as well as read.
 *
 * `ActivityResultContracts.OpenDocument` requests read only, and the grant a
 * picker returns cannot be widened afterwards — so a document opened through it
 * can never be saved back over, however the app later asks. Adding the flag at the
 * point the intent is built is the only place this can be fixed.
 *
 * A provider that has nothing writable to offer simply returns a read-only grant,
 * which is why this is a widening rather than a requirement: it costs nothing when
 * it cannot be honoured, and "Save a copy" covers that case.
 */
private class OpenWritableDocument : ActivityResultContracts.OpenDocument() {
    override fun createIntent(context: Context, input: Array<String>): Intent =
        super.createIntent(context, input).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
        }
}
