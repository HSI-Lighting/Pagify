package com.hsilighting.pagify.ui.model

import android.Manifest
import android.graphics.Bitmap
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack



import androidx.compose.material.icons.outlined.Info

import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.ui.components.CaptureEditor
import com.hsilighting.pagify.ui.components.CaptureMarkup
import com.hsilighting.pagify.ui.components.CaptureHint
import com.hsilighting.pagify.ui.components.captureOverlay
import kotlin.math.abs

/**
 * A part, turned with a finger.
 *
 * # Two resolutions, and why
 *
 * Rasterising a 465-face part takes about 27 ms at 320 x 320 and rather more at
 * the size of a phone screen — fine for a still, far too slow to follow a
 * finger. So a gesture draws at a fraction of the resolution and the full one
 * arrives when the finger lifts, the same trade the PDF reader already makes
 * while a page is being pinched. A moving picture that is slightly soft reads
 * as motion; a sharp one that lags reads as a broken app.
 *
 * # Where the drawing happens
 *
 * In Rust, into a `Bitmap`, exactly as a PDF page is drawn. There is no
 * `SurfaceView`, no GL context and no GPU: the 3D view is an `Image` whose
 * contents somebody else filled in, which is why it needed no new plumbing on
 * either platform.
 */
@Composable
fun ModelViewer(
    state: ModelViewerState,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var showingDetails by rememberSaveable { mutableStateOf(false) }
    var framing by rememberSaveable { mutableStateOf(false) }
    var lasso by rememberSaveable { mutableStateOf(false) }

    Column(modifier.fillMaxSize().background(BACKDROP)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 4.dp, end = 8.dp, top = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(
                    Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = "Back",
                    tint = Color.White,
                )
            }
            Column(Modifier.weight(1f)) {
                Text(
                    state.name,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = Color.White,
                    maxLines = 1,
                )
                state.subtitle?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = Color(0xFFB8BEC6))
                }
            }
            IconButton(onClick = { showingDetails = !showingDetails }) {
                Icon(
                    Icons.Outlined.Info,
                    contentDescription = if (showingDetails) "Hide the details" else "Show the details",
                    tint = if (showingDetails) Color(0xFF8AB4F8) else Color.White,
                )
            }
        }

        Box(Modifier.weight(1f).fillMaxWidth()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                when {
                    state.error != null -> Message(state.error!!)
                    state.loading -> CircularProgressIndicator(color = Color.White)
                    else -> Surface(state)
                }
            }

            // **Over the model, not instead of it.** The numbers describe what
            // is on screen, so putting them beside it means reading one while
            // looking at the other. A panel that can be dismissed keeps both.
            if (showingDetails && state.details.isNotEmpty()) {
                Details(
                    state.details,
                    Modifier.align(Alignment.TopStart).padding(12.dp),
                )
            }

            // **A layer of its own, above the model.** The same overlay the
            // reader uses on a page, so the tool behaves identically in both
            // places: drag a box or trace a ring, everything outside dims, and
            // a drag too small to mean anything is ignored. Above rather than
            // beside, because the gesture underneath turns the part — one
            // surface cannot both orbit and select, and letting it try is how
            // a selection ends up spinning the model it was framing.
            if (framing) {
                Box(
                    Modifier
                        .matchParentSize()
                        .captureOverlay(lasso = lasso) { box, ring ->
                            state.takeRegion(box, ring)
                            framing = false
                        },
                )
                // The reader's own words, not a second phrasing of them.
                CaptureHint(
                    lasso = lasso,
                    modifier = Modifier.align(Alignment.TopCenter).padding(top = 16.dp),
                )
            }

            // The ribbon floats over the model, along the bottom, where the
            // reader's is. Nothing is laid out around it, so turning the part
            // is not interrupted by the tools appearing and disappearing.
            ModelRibbon(
                framing = framing,
                lasso = lasso,
                onFrame = { framing = it },
                onLasso = { lasso = it },
                onFit = state::fit,
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 20.dp)
                    .navigationBarsPadding(),
            )
        }

        // **What was left out, on the screen rather than in a log.** A part
        // drawn with faces missing looks like the part. This line is the only
        // thing that says otherwise.
        state.warning?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = Color(0xFFE7A94B),
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
            )
        }
    }

    Captures(state)
}

/**
 * The picture the camera button takes, and getting it out of the app.
 *
 * Kept here rather than passed up to the activity: nothing about a capture of
 * a model concerns the rest of the app, and the permission below is the only
 * part that needs the system at all.
 */
@Composable
private fun Captures(state: ModelViewerState) {
    val context = LocalContext.current

    // The reader's editor, on a picture of a part. Same tools, same marks; the
    // only thing a model needed was somewhere to keep them that is not a
    // document, and a way to burn them in that is not a page re-render.
    val markup = remember(state.taken) { CaptureMarkup() }

    val storage = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) state.savePicture(context, markup.marks) else state.noteStorageRefused()
    }

    state.preview?.let { preview ->
        CaptureEditor(
            preview = preview,
            isCapturing = state.capturing,
            markup = markup.marks,
            markupTool = markup.tool,
            markupArmed = markup.armed,
            onDisarmMarkup = markup::disarm,
            markupColor = markup.colour,
            markupSize = markup.size,
            markupStyle = markup.style,
            onMarkupStyle = markup::style,
            onScaleChange = state::chooseScale,
            onFormatChange = state::chooseFormat,
            // A picture of a part is all part: nothing around it for a fill.
            fill = CaptureFill.PAGE,
            onFillChange = {},
            onMarkupTool = markup::use,
            onMarkupColor = markup::colour,
            onMarkupSize = markup::size,
            textFont = markup.textFont,
            textSizePoints = markup.textSizePoints,
            textCurveDegrees = markup.textCurveDegrees,
            onTextFont = markup::font,
            onTextSize = markup::textSize,
            onTextCurve = markup::textCurve,
            onCommitMarkup = markup::add,
            onRecogniseMarkup = { markup.add(MarkupShape.Freehand(it)) },
            onUndoMarkup = markup::undo,
            onMoveMarkup = markup::move,
            onSelectMarkup = markup::select,
            onScaleMarkup = markup::scaleSelected,
            onRewriteMarkup = markup::rewrite,
            onEraseMarkup = markup::erase,
            selectedMarkup = markup.selected,
            onSaveToGallery = {
                if (CaptureExport.galleryNeedsPermission()) {
                    storage.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                } else {
                    state.savePicture(context, markup.marks)
                }
            },
            onShare = { state.sharePicture(context, markup.marks) },
            onCopy = { state.copyPicture(context, markup.marks) },
            onDismiss = state::discardCapture,
            // A model has no pages either.
            origin = state.name,
        )
    }

    LaunchedEffect(state.shareRequest) {
        val uri = state.shareRequest ?: return@LaunchedEffect
        context.startActivity(CaptureExport.shareIntent(uri, state.captureFormat))
        state.shareRaised()
    }

    LaunchedEffect(state.message) {
        val note = state.message ?: return@LaunchedEffect
        Toast.makeText(context, note, Toast.LENGTH_SHORT).show()
        state.messageShown()
    }
}

@Composable
private fun Message(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge,
        color = Color(0xFFD8DDE4),
        modifier = Modifier.padding(32.dp),
    )
}

@Composable
private fun Surface(state: ModelViewerState) {
    Box(
        Modifier
            .fillMaxSize()
            // The viewer cannot draw until it knows how large it is, and the
            // only honest source of that is the layout.
            .onSizeChanged { state.resize(it.width, it.height) }
            .pointerInput(state) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    state.beginGesture()

                    var lastApart = 0f
                    while (true) {
                        val event = awaitPointerEvent()
                        val touching = event.changes.filter { it.pressed }
                        if (touching.isEmpty()) break

                        if (touching.size >= 2) {
                            // Two fingers pinch and slide. Turning stays a
                            // one-finger gesture, so a pinch can never spin
                            // the part by accident.
                            val apart = (touching[0].position - touching[1].position).getDistance()
                            if (lastApart > 0f && apart > 0f) state.zoom(apart / lastApart)
                            lastApart = apart
                            val slide = touching[0].positionChange()
                            state.pan(slide.x / size.width, slide.y / size.height)
                        } else {
                            lastApart = 0f
                            // **Both against the same length.** Dividing across by
                            // the width and down by the height makes the two
                            // axes turn at different rates per finger-pixel on
                            // any screen that is not square, so a diagonal drag
                            // arrives at the engine skewed.
                            val reach = size.width.coerceAtLeast(1)
                            val moved = touching[0].positionChange()
                            if (abs(moved.x) > 0.01f || abs(moved.y) > 0.01f) {
                                state.orbit(moved.x / reach, moved.y / reach)
                            }
                        }
                        touching.forEach { it.consume() }
                    }
                    state.endGesture()
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        state.picture?.let { picture ->
            Image(
                bitmap = picture.bitmap.asImageBitmap(),
                contentDescription = "The model, which can be turned with a finger",
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}
/** The dark ground the part is drawn on, matching the rasteriser's own. */
private val BACKDROP = Color(0xFF1E2024)

/** How much of the full resolution a gesture draws at. */
internal const val PROXY_FRACTION = 0.4f

/** Bitmaps are created here so the viewer and its tests agree on the format. */
internal fun modelBitmap(width: Int, height: Int): Bitmap =
    Bitmap.createBitmap(width.coerceAtLeast(1), height.coerceAtLeast(1), Bitmap.Config.ARGB_8888)

/**
 * What the part is, as a table.
 *
 * **A table, not a sentence.** These are figures somebody compares — how many
 * faces against how many were drawn, how many of each surface kind — and
 * comparing is what prose is worst at. Two columns with the numbers right
 * aligned and lined up on their digits can be read down in a second; the same
 * facts in a paragraph have to be read through.
 *
 * Grouped under headings, because the eye finds the group before the row.
 */
@Composable
private fun Details(rows: List<DetailRow>, modifier: Modifier = Modifier) {
    Column(
        modifier
            .background(Color(0xE6141619), RoundedCornerShape(12.dp))
            .padding(horizontal = 14.dp, vertical = 10.dp),
    ) {
        rows.forEach { row ->
            when (row) {
                is DetailRow.Heading -> Text(
                    row.text.uppercase(),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = Color(0xFF7C8592),
                    modifier = Modifier.padding(top = 8.dp, bottom = 3.dp),
                )

                is DetailRow.Item -> Row(
                    Modifier.padding(vertical = 1.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        row.label,
                        style = MaterialTheme.typography.bodySmall,
                        color = Color(0xFFB3BAC4),
                        modifier = Modifier.width(142.dp),
                    )
                    Text(
                        row.value,
                        // Right aligned on tabular figures, so the digits of one
                        // row sit above the digits of the next. That alignment
                        // is most of what makes a column readable at a glance.
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontFeatureSettings = "tnum",
                        ),
                        fontWeight = FontWeight.Medium,
                        color = Color.White,
                        textAlign = TextAlign.End,
                        modifier = Modifier.width(96.dp),
                    )
                }
            }
        }
    }
}
