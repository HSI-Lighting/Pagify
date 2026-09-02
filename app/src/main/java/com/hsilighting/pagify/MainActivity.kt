package com.hsilighting.pagify

import java.io.File
import androidx.core.content.FileProvider
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.hsilighting.pagify.core.BlankFrameDetector
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.isDark
import com.hsilighting.pagify.ui.components.PageAction
import com.hsilighting.pagify.ui.PagifyApp
import com.hsilighting.pagify.ui.contacts.CardReviewState
import com.hsilighting.pagify.ui.components.BlankPageSheet
import com.hsilighting.pagify.ui.components.NewDocumentChooser
import com.hsilighting.pagify.ui.components.LeavePrompt
import com.hsilighting.pagify.ui.reader.LeaveIntent
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
                val contacts by viewModel.contacts.collectAsStateWithLifecycle()
                val contactGroups by viewModel.contactGroups.collectAsStateWithLifecycle()
                val groupMemberships by viewModel.groupMemberships.collectAsStateWithLifecycle()
                val suggestedGroup by viewModel.lastFiled.collectAsStateWithLifecycle()
                val pendingFiling by viewModel.pendingFiling.collectAsStateWithLifecycle()
                val pendingReview by viewModel.pendingReview.collectAsStateWithLifecycle()

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
                        keepAccessTo(uri)
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
                ) { uri ->
                    if (uri == null) {
                        viewModel.copyDestinationAbandoned()
                    } else {
                        keepAccessTo(uri)
                        viewModel.saveCopyTo(uri)
                    }
                }

                val openPicker = remember { { picker.launch(arrayOf(PDF_MIME_TYPE)) } }

                /**
                 * The group a scan in flight belongs to.
                 *
                 * `rememberSaveable`, because the camera is another activity and
                 * this process can be killed while it is in front. Held in a
                 * plain field it came back null, the card was saved unfiled, and
                 * the user was asked a question they had already answered by
                 * standing inside the group — "sometimes it does not add it".
                 */
                var pendingScanGroup by rememberSaveable { mutableStateOf<Long?>(null) }

                /**
                 * A business card already on the device.
                 *
                 * For one photographed earlier, or sent by somebody else. Needs no
                 * permission of any kind.
                 */
                val cardPicker = rememberLauncherForActivityResult(
                    ActivityResultContracts.OpenDocument(),
                ) { uri -> uri?.let { viewModel.scanCard(it, pendingScanGroup) } }

                /**
                 * A card photographed here and now.
                 *
                 * The system camera app rather than CameraX: no dependency, no
                 * preview to build, and — because this app does not declare
                 * `CAMERA` in its manifest — **no permission prompt**.
                 * `TakePicture` requires the permission only from an app that has
                 * declared it, so declaring it in order to be thorough would
                 * introduce the very prompt that not declaring it avoids.
                 *
                 * The destination is held here across the trip to the camera,
                 * which is another activity and can take this process down with it
                 * on a low-memory device. On that path the photo is lost and the
                 * user takes it again — the alternative, restoring a `Uri` whose
                 * file may no longer exist, fails less honestly.
                 */
                var cardPhoto by remember { mutableStateOf<Uri?>(null) }
                val cardCamera = rememberLauncherForActivityResult(
                    ActivityResultContracts.TakePicture(),
                ) { taken ->
                    // False means cancelled or failed. The empty file it leaves
                    // behind is not worth recognising, and the cache clears itself.
                    if (taken) cardPhoto?.let { viewModel.scanCard(it, pendingScanGroup) }
                    cardPhoto = null
                }

                /**
                 * Where chosen pages are written out.
                 *
                 * The same contract as "Save a copy": the reader names it and
                 * says where it goes. No persisted grant is taken — an export is
                 * a file handed to somebody else, not one this app comes back to.
                 */
                val exportPicker = rememberLauncherForActivityResult(
                    ActivityResultContracts.CreateDocument(PDF_MIME_TYPE),
                ) { uri ->
                    if (uri == null) viewModel.exportAbandoned()
                    else viewModel.exportPagesTo(uri)
                }

                /** The file to take pages from. Opened read-only and closed after. */
                val importPicker = rememberLauncherForActivityResult(
                    ActivityResultContracts.OpenDocument(),
                ) { uri -> uri?.let(viewModel::openImportSource) }

                /**
                 * Where a new blank document is written.
                 *
                 * The same contract as "Save a copy": the reader names it and
                 * says where it goes, which is both the only way to write outside
                 * the sandbox and what makes it findable again afterwards.
                 */
                val createPicker = rememberLauncherForActivityResult(
                    ActivityResultContracts.CreateDocument(PDF_MIME_TYPE),
                ) { uri ->
                    if (uri == null) {
                        viewModel.newDocumentAbandoned()
                    } else {
                        // The same grant the open picker takes, for the same reason
                        // and one more: a document made here goes straight into the
                        // library, and without it that entry is dead the next time
                        // the app starts.
                        keepAccessTo(uri)
                        viewModel.createNewDocument(uri)
                    }
                }

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

                /**
                 * Leaving, once the reader has answered for their unsaved work.
                 *
                 * Acted on here rather than in the view model because one of the two
                 * destinations is the system file picker, which only an activity can
                 * open.
                 */
                state.leaveNow?.let { intent ->
                    LaunchedEffect(intent) {
                        viewModel.leftDocument()
                        when (intent) {
                            LeaveIntent.Library -> viewModel.returnToLibrary()
                            LeaveIntent.AnotherDocument -> openPicker()
                        }
                    }
                }

                if (state.showNewDocumentChooser) {
                    NewDocumentChooser(
                        onBlankPages = viewModel::describeNewDocument,
                        onOpenFile = {
                            viewModel.dismissNewDocument()
                            openPicker()
                        },
                        onDismiss = viewModel::dismissNewDocument,
                    )
                }

                if (state.showNewDocumentSheet) {
                    BlankPageSheet(
                        // No page to match: this is the first one.
                        template = null,
                        newDocument = true,
                        onAdd = { sheet ->
                            createPicker.launch(viewModel.newDocumentDescribed(sheet))
                        },
                        onDismiss = viewModel::dismissNewDocument,
                    )
                }

                state.pendingLeave?.let { intent ->
                    LeavePrompt(
                        intent = intent,
                        onSave = viewModel::saveThenLeave,
                        onSaveAs = {
                            viewModel.leaveViaCopy()
                            copyPicker.launch(suggestedCopyName(state.documentName))
                        },
                        onExit = viewModel::leaveWithoutSaving,
                        onClose = viewModel::cancelLeaving,
                    )
                }

                // Sharing an exported contact. Written to the cache and handed
                // over as a content:// URI with a grant, never a file path.
                val shareContact: (String, String) -> Unit = { name, vcard ->
                    runCatching { shareVCard(name, vcard) }
                        .onFailure {
                            Toast.makeText(
                                this,
                                it.message ?: "The contact could not be shared.",
                                Toast.LENGTH_LONG,
                            ).show()
                        }
                }

                PagifyApp(
                    state = state,
                    recents = recents,
                    onOpenRecent = { viewModel.open(it.uri.toUri()) },
                    onForgetRecent = { viewModel.forgetDocument(it.uri) },
                    onPickDocument = { viewModel.showNewDocumentChooser(true) },
                    onClearLibrary = viewModel::clearLibrary,
                    onShowThumbnails = viewModel::setThumbnails,
                    settings = settings,
                    onThemeChange = viewModel::setTheme,
                    onShowViewfinder = viewModel::setShowViewfinder,
                    onToggleRecording = recordingToast,
                    onReturnToLibrary = { viewModel.askBeforeLeaving(LeaveIntent.Library) },
                    contacts = contacts,
                    contactGroups = contactGroups,
                    groupMemberships = groupMemberships,
                    suggestedGroup = suggestedGroup,
                    onCreateGroup = { viewModel.createGroup(it, eventDate = null) },
                    review = pendingReview?.let { pending ->
                        CardReviewState(
                            imageUri = pending.imageUri,
                            reading = pending.readings[pending.at],
                            position = pending.at,
                            total = pending.readings.size,
                        )
                    },
                    onKeepReviewed = viewModel::keepReviewedCard,
                    onSkipReviewed = viewModel::skipReviewedCard,
                    pendingFilingLabel = pendingFiling?.label,
                    onFileScanned = { viewModel.fileScanned(it) },
                    onCreateGroupForScan = viewModel::createGroupForScan,
                    onSkipFiling = viewModel::dismissFiling,
                    onRenameGroup = viewModel::renameGroup,
                    onDeleteGroup = viewModel::deleteGroup,
                    onExportGroup = { group ->
                        viewModel.exportGroup(group) { name, vcard ->
                            shareContact(name, vcard)
                        }
                    },
                    onRemoveFromGroup = viewModel::removeFromGroup,
                    onAddToGroupPicked = viewModel::addToGroupPicked,
                    onCreateGroupWith = viewModel::createGroupWith,
                    onScanCard = { group ->
                        pendingScanGroup = group
                        cardPicker.launch(arrayOf("image/*"))
                    },
                    onPhotographCard = { group ->
                        pendingScanGroup = group
                        val destination = runCatching { newCardPhoto() }.getOrNull()
                        if (destination == null) {
                            viewModel.report("There was nowhere to save the photo.")
                        } else {
                            cardPhoto = destination
                            // No camera app at all is rare but real, and a crash
                            // is a poor way to say so.
                            runCatching { cardCamera.launch(destination) }.onFailure {
                                cardPhoto = null
                                viewModel.report("This device has no camera app.")
                            }
                        }
                    },
                    onExportContact = { contact ->
                        viewModel.exportContact(contact) { vcard ->
                            shareContact(contact.displayName, vcard)
                        }
                    },
                    onDeleteContact = viewModel::deleteContact,
                    onSaveContact = viewModel::updateContact,
                    onDeleteContacts = viewModel::deleteContacts,
                    onDeleteGroups = viewModel::deleteGroups,
                    onExportSelected = { chosen ->
                        viewModel.exportSelected(chosen) { name, vcard ->
                            shareContact(name, vcard)
                        }
                    },
                    onMessageShown = viewModel::messageShown,
                ) {
                    PdfReaderScreen(
                        state = state,
                        onPickDocument = { viewModel.askBeforeLeaving(LeaveIntent.AnotherDocument) },
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
                        onShowBlankPage = { viewModel.showBlankPageSheet(true) },
                        onAddBlankPage = viewModel::insertBlankPage,
                        onDismissBlankPage = { viewModel.showBlankPageSheet(false) },
                        onDeleteCurrentPage = viewModel::deleteCurrentPage,
                        onPageAction = { action ->
                            when (action) {
                                is PageAction.Select -> viewModel.selectPage(action.index)
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
                        onExportPages = { viewModel.choosePagesToExport(true) },
                        onImportPages = { importPicker.launch(arrayOf(PDF_MIME_TYPE)) },
                        onPagesChosenToExport = { pages ->
                            exportPicker.launch(viewModel.pagesChosenToExport(pages))
                        },
                        onCancelExport = { viewModel.choosePagesToExport(false) },
                        onPagesChosenToImport = viewModel::importChosenPages,
                        onCancelImport = viewModel::closeImportSource,
                        importSourcePageSize = viewModel::importSourcePageSize,
                        importSourceRenderer = viewModel::renderImportSourcePage,
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
                        onTextFont = viewModel::setTextFont,
                        onTextSize = viewModel::setTextSize,
                        onTextCurve = viewModel::setTextCurve,
                        onPlaceText = viewModel::beginText,
                        onMoveText = viewModel::moveMark,
                        onSelectText = viewModel::selectText,
                        onScaleText = viewModel::scaleSelectedText,
                        onEditText = viewModel::editText,
                        onTurnZoomedPage = viewModel::turnZoomedPage,
                        onCommitText = viewModel::commitText,
                        onCancelText = viewModel::cancelText,
                        onMarkupTool = viewModel::setMarkupTool,
                        onDisarmMarkup = viewModel::disarmMarkup,
                        onMarkupColor = viewModel::setMarkupColor,
                        onMarkupSize = viewModel::setMarkupSize,
                        onMarkupStyle = viewModel::setMarkupStyle,
                        onCommitMarkup = viewModel::addMarkup,
                        onRecogniseMarkup = viewModel::recogniseAndAddMarkup,
                        onUndoMarkup = viewModel::undoMarkup,
                        onMoveMarkup = viewModel::moveMarkup,
                        onSelectMarkup = viewModel::selectMarkup,
                        onScaleMarkup = viewModel::scaleSelectedMarkup,
                        onRewriteMarkup = viewModel::rewriteMarkup,
                        onEraseMarkup = viewModel::eraseMarkup,
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

        /** What a vCard is, so a share sheet offers contact apps first. */
        const val VCARD_MIME_TYPE = "text/x-vcard"

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

    /**
     * Somewhere for the camera app to write a card photograph.
     *
     * Its own cache directory, not the one page captures use: a grant handed to
     * the camera should not reach a document the user never meant to share. The
     * FileProvider authority is declared in the manifest and the path in
     * `capture_paths.xml`; both have to agree with this or the camera is handed a
     * URI it cannot write to.
     *
     * The cache is chosen deliberately. The photograph is a means to a contact,
     * not a thing the user asked to keep, and the system reclaiming it later is
     * the right outcome rather than a loss.
     */
    private fun newCardPhoto(): Uri {
        val directory = File(cacheDir, "cards").apply { mkdirs() }
        val file = File(directory, "card-${System.currentTimeMillis()}.jpg")
        return FileProvider.getUriForFile(this, "$packageName.captures", file)
    }

    /**
     * Hold on to a document past this process.
     *
     * Without it the grant expires when the app does, so a document reopened
     * after a low-memory kill — or listed in the library after a restart — fails
     * with a permission denial on a file the reader plainly chose.
     *
     * Write is taken alongside read because saving edits back to the file needs
     * it and it cannot be asked for later: the grant is fixed when the picker
     * returns.
     */
    private fun keepAccessTo(uri: Uri) {
        runCatching {
            contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
        }.onFailure {
            // A read-only provider refuses the write half. The document still
            // opens, and "Save a copy" is the way out; failing over it is worse.
            runCatching {
                contentResolver.takePersistableUriPermission(
                    uri,
                    Intent.FLAG_GRANT_READ_URI_PERMISSION,
                )
            }
        }
    }

    /**
     * Hand an exported contact to whatever the reader wants to send it with.
     *
     * Written into the cache and shared as a `content://` URI with a grant,
     * never a `file://` path — the latter throws on anything modern and would
     * hand out a path into our own storage besides.
     *
     * The file is named after the contact so it arrives somewhere recognisable
     * rather than as `contact.vcf` among a dozen others.
     */
    private fun shareVCard(name: String, vcard: String) {
        val directory = File(cacheDir, "contacts").apply { mkdirs() }
        // Anything a file name cannot hold becomes an underscore. A company
        // with a slash in it would otherwise write outside the directory.
        val safe = name.replace(Regex("[^A-Za-z0-9 _-]"), "_").trim().ifBlank { "Contact" }
        val file = File(directory, "$safe.vcf").apply { writeText(vcard) }

        val uri = FileProvider.getUriForFile(this, "$packageName.captures", file)
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = VCARD_MIME_TYPE
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        startActivity(Intent.createChooser(intent, "Send $safe"))
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
