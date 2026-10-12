package com.hsilighting.pagify

import java.io.File
import androidx.core.content.FileProvider
import android.Manifest.permission.WRITE_EXTERNAL_STORAGE
import android.content.Context
import android.content.Intent
import android.os.Build
import android.net.Uri
import android.util.Log
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.LaunchedEffect
import com.hsilighting.pagify.ui.drawing.DrawingViewer
import com.hsilighting.pagify.ui.drawing.DrawingViewerState
import com.hsilighting.pagify.ui.model.ModelViewer
import com.hsilighting.pagify.ui.model.ModelViewerState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.hsilighting.pagify.core.BlankFrameDetector
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.core.RecentKind
import com.hsilighting.pagify.core.kindOfFile
import com.hsilighting.pagify.core.modelCacheKey
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

                // The actual routing is below `showModel`/`showDrawing`, which this
                // needs and which are declared further down this same function.

                // **The 3D viewer's way in, on this branch only.**
                //
                // A STEP file is copied into the app's own storage before
                // being opened. The engine reads a path, and a content URI
                // from the picker is not one -- it is a handle to something
                // another app is holding, which may be on a network share or
                // inside a zip.
                var modelState by remember { mutableStateOf<ModelViewerState?>(null) }
                val modelScope = rememberCoroutineScope()

                /**
                 * Open a model, from the picker or from the library.
                 *
                 * The copy is kept permanently (see `copyForReading`), so a
                 * remembered model reopens from it directly rather than being
                 * re-copied from its uri — which matters because that uri's
                 * read grant is not guaranteed to still be good by then.
                 */
                // Takes the display name as a parameter rather than reading
                // `copied.name`: the cache file underneath is keyed on the uri,
                // not on this name (see `copyForReading`), so the two can differ
                // and the title shown here must stay the human-readable one.
                val showModel: (Uri, String) -> Boolean = { uri, displayName ->
                    val copied = copyForReading(uri)
                    if (copied != null) {
                        modelState = ModelViewerState(displayName, modelScope)
                            .also { it.open(copied.absolutePath) }
                    }
                    copied != null
                }

                val modelPicker = rememberLauncherForActivityResult(
                    OpenReadableDocument(),
                ) { uri ->
                    if (uri != null) {
                        keepReadAccessTo(uri)
                        val name = nameOf(uri)
                        if (showModel(uri, name)) {
                            // Remembered only once it has opened, the same rule
                            // the reader follows: a file that failed is not
                            // something to offer again as if it worked.
                            viewModel.rememberModel(
                                uri = uri.toString(),
                                name = name,
                                sizeBytes = sizeOf(uri),
                            )
                        }
                    }
                }

                var drawingState by remember { mutableStateOf<DrawingViewerState?>(null) }

                /**
                 * Open a drawing, from the picker or from the library.
                 *
                 * **The extension has to survive the copy.** The engine chooses
                 * its reader by extension, so `copyForReading` keeps the
                 * original one on the cache file even though it renames
                 * everything ahead of it — a copy written as `.bin` would
                 * arrive as a DXF and fail on the first byte.
                 */
                val showDrawing: (Uri, String) -> Boolean = { uri, displayName ->
                    val copied = copyForReading(uri)
                    if (copied != null) {
                        drawingState = DrawingViewerState(displayName, modelScope)
                            .also { it.open(copied.absolutePath) }
                    }
                    copied != null
                }

                val drawingPicker = rememberLauncherForActivityResult(
                    OpenReadableDocument(),
                ) { uri ->
                    if (uri != null) {
                        keepReadAccessTo(uri)
                        val name = nameOf(uri)
                        if (showDrawing(uri, name)) {
                            viewModel.rememberDrawing(
                                uri = uri.toString(),
                                name = name,
                                sizeBytes = sizeOf(uri),
                            )
                        }
                    }
                }

                /**
                 * A file handed to Pagify from outside — another app's share
                 * sheet, or Android's "Open with" chooser.
                 *
                 * **Routed by kind, the same way the in-app picker already is.**
                 * This used to go straight to `viewModel.open(uri)` — the PDF
                 * reader's own open call — whatever the file actually was,
                 * because nothing outside the app could reach a DWG, DXF or
                 * STEP file before the manifest offered Pagify for them. Now
                 * that it does, a drawing or a model arriving this way needs
                 * the same `copyForReading` + remember-on-success treatment the
                 * picker gives it, not a PDF reader trying to parse a STEP file
                 * and failing on the first byte.
                 */
                LaunchedEffect(incoming) {
                    incoming?.let { uri ->
                        keepReadAccessTo(uri)
                        val name = nameOf(uri)
                        when (kindOfFile(name)) {
                            RecentKind.Model ->
                                if (showModel(uri, name)) {
                                    viewModel.rememberModel(uri.toString(), name, sizeOf(uri))
                                }
                            RecentKind.Drawing ->
                                if (showDrawing(uri, name)) {
                                    viewModel.rememberDrawing(uri.toString(), name, sizeOf(uri))
                                }
                            RecentKind.Document -> viewModel.open(uri)
                        }
                        // Cleared so a configuration change does not reopen it and
                        // discard the page or view the user had scrolled to.
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
                        // **The file says which viewer it wants.** One "Open a
                        // file" for four formats, routed by name — asking
                        // somebody to declare in advance whether they are
                        // opening a PDF, a part or a drawing is asking them to
                        // answer a question the file has already answered.
                        val name = nameOf(uri)
                        when (kindOfFile(name)) {
                            RecentKind.Model ->
                                if (showModel(uri, name)) {
                                    viewModel.rememberModel(uri.toString(), name, sizeOf(uri))
                                }
                            RecentKind.Drawing ->
                                if (showDrawing(uri, name)) {
                                    viewModel.rememberDrawing(uri.toString(), name, sizeOf(uri))
                                }
                            // The reader remembers its own, once it has opened.
                            RecentKind.Document -> viewModel.open(uri)
                        }
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

                // **Everything, not just PDFs.** Most providers hand a DWG or a
                // STEP file over as `application/octet-stream`, so filtering
                // the picker by type would hide the very files this now opens.
                // Which viewer gets it is decided from the name afterwards.
                val openPicker = remember { { picker.launch(arrayOf("*/*")) } }

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

                // The viewer takes the whole screen while a model is open,
                // and back closes it -- releasing the native handle, which is
                // megabytes of mesh that will not collect itself.
                modelState?.let { model ->
                    BackHandler { model.close(); modelState = null }
                    ModelViewer(
                        state = model,
                        onBack = { model.close(); modelState = null },
                    )
                    return@PagifyTheme
                }

                // A drawing takes the screen the same way, and back releases it
                // for the same reason: a plan expanded to a few hundred
                // thousand shapes is native memory nothing else will reclaim.
                drawingState?.let { drawing ->
                    BackHandler { drawing.close(); drawingState = null }
                    DrawingViewer(
                        state = drawing,
                        onBack = { drawing.close(); drawingState = null },
                    )
                    return@PagifyTheme
                }

                PagifyApp(
                    state = state,
                    recents = recents,
                    onOpenRecent = { recent ->
                        // The list holds both kinds now, and they reopen in
                        // different screens. Reading the kind rather than
                        // guessing from the file name: a supplier who names a
                        // part "drawing.stp" should still get the 3D viewer.
                        //
                        // **A model or a drawing can still fail here, for a
                        // file that was never opened successfully even once.**
                        // `copyForReading` reopens from its permanent copy when
                        // one already exists, which is the normal case and
                        // needs no uri access at all — but the very first open
                        // of a file shared from another app can itself fail
                        // (that app's read grant can be gone by the time this
                        // runs), and then there is no copy here to fall back
                        // on. `showModel`/`showDrawing` already swallow that
                        // failure internally (a dead grant throwing
                        // `SecurityException` mid-copy is not this app's bug to
                        // surface as a crash), but swallowing it this far up as
                        // well left somebody tapping a drawing and watching
                        // nothing happen, with no way to tell "this needs
                        // reopening from Telegram again" from "I didn't
                        // actually tap it."
                        val opened = when (recent.kind) {
                            RecentKind.Model -> showModel(recent.uri.toUri(), recent.name)
                            RecentKind.Drawing -> showDrawing(recent.uri.toUri(), recent.name)
                            RecentKind.Document -> {
                                viewModel.open(recent.uri.toUri())
                                true
                            }
                        }
                        if (!opened) {
                            Toast.makeText(
                                this,
                                "Pagify no longer has permission to open " +
                                    "\"${recent.name}\". Open it again from " +
                                    "where you found it.",
                                Toast.LENGTH_LONG,
                            ).show()
                        }
                    },
                    onForgetRecent = { viewModel.forgetDocument(it.uri) },
                    onShareRecent = ::shareDocument,
                    onPickDocument = { viewModel.showNewDocumentChooser(true) },
                    onClearLibrary = viewModel::clearLibrary,
                    onShowThumbnails = viewModel::setThumbnails,
                    settings = settings,
                    onThemeChange = viewModel::setTheme,
                    onCardTextScale = viewModel::setCardTextScale,
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
                    cardTextScale = settings.cardTextScale,
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
                    onCreateContact = viewModel::updateContact,
                    onSaveProgress = viewModel::updateProgress,
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
                        onShareDocument = {
                            viewModel.openDocumentUri?.let {
                                shareDocumentUri(it, state.documentName)
                            }
                        },
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

    /**
     * The file a VIEW or a SEND intent is actually offering, which are not
     * the same field.
     *
     * **A SEND intent never carries its payload in `data`.** VIEW does — the
     * URI being viewed is the Intent's own data. SEND puts the file in the
     * `EXTRA_STREAM` extra instead, because a share intent's `data`/`type`
     * pair is conventionally reserved for describing what's being shared
     * (a `text/plain` share with a caption, say), not for carrying it. This
     * code accepted SEND and read `.data` from it regardless, which is the
     * reason "share a PDF to Pagify" from another app's share sheet has
     * never actually opened anything — `.data` on a SEND intent is normally
     * null, so the whole open silently did nothing. Surfaced while adding a
     * SEND filter for CAD files, where it would have repeated identically.
     */
    private fun viewableUri(intent: Intent?): Uri? {
        intent ?: return null
        return when (intent.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
            else -> null
        }
    }

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

    /**
     * Hand a library document to another app.
     *
     * **The URI is forwarded rather than the file copied.** A recent document is
     * already a content URI this app holds a persistent grant on, so it can be
     * passed along with a read grant attached — no second copy of a
     * fifty-megabyte PDF in the cache, and nothing to clean up afterwards.
     *
     * The grant is the whole subtlety. Without `FLAG_GRANT_READ_URI_PERMISSION`
     * the receiving app gets a URI it cannot open, and what the user sees is a
     * mail draft whose attachment fails at send — long after the moment anybody
     * would connect it back to this.
     */
    private fun shareDocument(document: RecentDocument) {
        val uri = runCatching { document.uri.toUri() }.getOrNull() ?: run {
            Log.w("Pagify", "a library row had an unreadable uri")
            return
        }
        shareDocumentUri(uri, document.name)
    }

    /** The same, for whatever is open in the reader. */
    private fun shareDocumentUri(uri: Uri, name: String) {
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "application/pdf"
            putExtra(Intent.EXTRA_STREAM, uri)
            putExtra(Intent.EXTRA_TITLE, name)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        runCatching {
            startActivity(Intent.createChooser(intent, "Share $name"))
        }.onFailure { Log.w("Pagify", "nothing could accept the document", it) }
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
/**
 * A picker for something only ever read.
 *
 * Read and persistable, no write. A model is never edited, and asking for a
 * write grant a provider may refuse is a way to fail at opening something that
 * would have opened.
 */
/** What a provider calls a file, falling back to something usable. */
private fun ComponentActivity.nameOf(uri: android.net.Uri): String =
    runCatching {
        contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val column = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
            if (column >= 0 && cursor.moveToFirst()) cursor.getString(column) else null
        }
    }.getOrNull().orEmpty().ifBlank { uri.lastPathSegment ?: "model.stp" }

/** How large a file is, or zero where the provider will not say. */
private fun ComponentActivity.sizeOf(uri: android.net.Uri): Long =
    runCatching {
        contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val column = cursor.getColumnIndex(android.provider.OpenableColumns.SIZE)
            if (column >= 0 && cursor.moveToFirst()) cursor.getLong(column) else 0L
        } ?: 0L
    }.getOrDefault(0L)

/** Keep read access, so a model can be reopened from the library later. */
private fun ComponentActivity.keepReadAccessTo(uri: android.net.Uri) {
    runCatching {
        contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
}

private class OpenReadableDocument : ActivityResultContracts.OpenDocument() {
    override fun createIntent(context: Context, input: Array<String>): Intent =
        super.createIntent(context, input).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
        }
}

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

/**
 * Copy a picked file into the app's own storage so the engine can open it.
 *
 * **The engine takes a path, and a content URI is not one.** What the picker
 * returns is a handle to something another app is holding — it may be on a
 * network share, inside an archive, or on a drive that disconnects. Copying
 * first means the model is read from a real file that exists for as long as it
 * is being looked at, and it is the same reason the PDF path takes a file
 * descriptor rather than a name.
 */
// `filesDir`, not `cacheDir`: a model or drawing opened from another app's
// share often arrives with a permission grant that does not survive the
// Activity that received it, so a later reopen cannot always re-read the
// original uri. Keeping the copy in permanent storage means that reopen
// never has to — it is served from here instead, so the grant's lifetime
// no longer matters.
//
// The lookup below is keyed on the uri string itself, not on `nameOf(uri)`:
// that calls back into the content provider, which is exactly the thing a
// stale grant can no longer do, so keying on it would make the cache check
// fail for the same reason the read would have. The uri string needs no
// provider access at all and is the one thing guaranteed identical between
// the first open and every later one, since it is what the library stores.
private fun ComponentActivity.copyForReading(uri: android.net.Uri): File? = runCatching {
    val into = File(filesDir, "models").apply { mkdirs() }
    val key = modelCacheKey(uri.toString())
    into.listFiles { file -> file.name.startsWith("$key.") }
        ?.firstOrNull { it.length() > 0 }
        ?.let { return it }

    val ext = nameOf(uri).substringAfterLast('.', "")
    val file = File(into, if (ext.isBlank()) key else "$key.$ext")
    contentResolver.openInputStream(uri)?.use { source ->
        file.outputStream().use { sink -> source.copyTo(sink) }
    } ?: return null
    file
}.getOrNull()
