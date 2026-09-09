package com.hsilighting.pagify.ui.drawing

import android.Manifest
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material.icons.outlined.Straighten
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
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
import com.hsilighting.pagify.ui.components.ToolButton
import com.hsilighting.pagify.ui.components.captureOverlay
import com.hsilighting.pagify.ui.model.ModelRibbon

/**
 * A drawing, moved with a finger.
 *
 * **The same screen as the model viewer wherever it can be.** The ribbon, the
 * snapshot tool and the capture sheet are the model viewer's own composables,
 * not copies of them, so a drawing and a part behave identically and one change
 * fixes both. What differs is the gesture — a sheet pans and zooms, it does not
 * turn over — and the layer panel, which a solid has no equivalent of and a
 * forty-layer lighting plan is unreadable without.
 */
@Composable
fun DrawingViewer(
    state: DrawingViewerState,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var showingLayers by rememberSaveable { mutableStateOf(false) }
    var framing by rememberSaveable { mutableStateOf(false) }
    var lasso by rememberSaveable { mutableStateOf(false) }
    var measuring by rememberSaveable { mutableStateOf(false) }

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
            if (state.layers.isNotEmpty()) {
                IconButton(onClick = { showingLayers = !showingLayers }) {
                    Icon(
                        Icons.Outlined.Layers,
                        contentDescription = if (showingLayers) "Hide the layers" else "Show the layers",
                        tint = if (showingLayers) Color(0xFF8AB4F8) else Color.White,
                    )
                }
            }
        }

        Box(Modifier.weight(1f).fillMaxWidth()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                when {
                    state.error != null -> Message(state.error!!)
                    state.loading -> CircularProgressIndicator(color = Color.White)
                    else -> Sheet(state, onTap = if (measuring) state::measureAt else null)
                }
            }

            if (showingLayers && state.layers.isNotEmpty()) {
                Layers(
                    state,
                    Modifier.align(Alignment.TopStart).padding(12.dp),
                )
            }

            // A layer of its own above the sheet, for the same reason as in the
            // model viewer: the gesture underneath moves the drawing, and one
            // surface cannot both pan and select.
            if (framing) {
                Box(
                    Modifier
                        .matchParentSize()
                        .captureOverlay(lasso = lasso) { box, ring ->
                            state.takeRegion(box, ring)
                            framing = false
                        },
                )
                CaptureHint(
                    lasso = lasso,
                    modifier = Modifier.align(Alignment.TopCenter).padding(top = 16.dp),
                )
            }

            // The measurement, above the ribbon so a finger reaching for the
            // tools does not cover the number it just took.
            state.measurement?.let { taken ->
                MeasureReadout(
                    taken,
                    Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 92.dp)
                        .navigationBarsPadding(),
                )
            }

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
            ) {
                // **Only here.** A solid has no two points to measure between,
                // so the model viewer never grows this slot.
                ToolButton(
                    icon = Icons.Outlined.Straighten,
                    label = "Measure",
                    selected = measuring,
                    onClick = {
                        measuring = !measuring
                        if (!measuring) state.clearMeasurement()
                        // Two tools that both want a tap cannot both be armed.
                        if (measuring) framing = false
                    },
                )
            }
        }

        // **What was left out, on the screen rather than in a log.** A plan
        // without its room names and dimensions still looks like the plan,
        // which is exactly why it has to say so.
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

/** The sheet itself, and the gestures that move it. */
@Composable
private fun Sheet(state: DrawingViewerState, onTap: ((Float, Float) -> Unit)? = null) {
    Box(
        Modifier
            .fillMaxSize()
            .onSizeChanged { state.resize(it.width, it.height) }
            .pointerInput(state, onTap) {
                awaitEachGesture {
                    val first = awaitFirstDown(requireUnconsumed = false)
                    state.beginGesture()

                    // **A tap is a press that did not go anywhere.** Measuring
                    // has to live alongside panning rather than replacing it —
                    // a measuring tool that stops somebody moving the sheet
                    // makes them turn it off to reach the other end of a wall
                    // and lose the first point. So the gesture is watched, and
                    // only a press that stayed put counts as a tap.
                    val began = first.position
                    var wandered = false

                    var lastApart = 0f
                    while (true) {
                        val event = awaitPointerEvent()
                        val touching = event.changes.filter { it.pressed }
                        if (touching.isEmpty()) break

                        if (touching.size >= 2) {
                            val apart =
                                (touching[0].position - touching[1].position).getDistance()
                            // **About the middle of the pinch, not the middle
                            // of the screen.** Somebody spreads two fingers
                            // over a detail in the corner; without this the
                            // centre of the sheet comes up at them instead and
                            // the detail has to be dragged back into view.
                            val between = (touching[0].position + touching[1].position) / 2f
                            if (lastApart > 0f && apart > 0f) {
                                state.zoom(apart / lastApart, between.x, between.y)
                            }
                            lastApart = apart
                        } else {
                            lastApart = 0f
                            // **One finger pans, where on a model it turns.**
                            // A sheet has no other side, so the obvious gesture
                            // is the one that moves it about.
                            val moved = touching[0].positionChange()
                            if ((touching[0].position - began).getDistance() > STILL) {
                                wandered = true
                            }
                            // While the measuring tool is out, a drag still
                            // pans; it is only the tap that is spoken for.
                            state.pan(moved.x / size.width, moved.y / size.height)
                        }
                        touching.forEach { it.consume() }
                    }

                    state.endGesture()
                    if (!wandered && onTap != null) onTap(began.x, began.y)
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        state.picture?.let { frame ->
            Image(
                bitmap = frame.bitmap.asImageBitmap(),
                contentDescription = "The drawing",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Fit,
            )
        }
    }
}

/**
 * Which layers are showing.
 *
 * **Not optional.** A forty-layer lighting layout is unreadable with all of it
 * on at once — the grid, the dimensions, the furniture and the fittings drawn
 * over each other — and turning one off is the whole difference between a
 * drawing somebody can use and a grey mess.
 */
@Composable
private fun Layers(state: DrawingViewerState, modifier: Modifier = Modifier) {
    Box(
        modifier
            .width(260.dp)
            .heightIn(max = 380.dp)
            .background(Color(0xE6161A1F), RoundedCornerShape(12.dp))
            .padding(vertical = 8.dp),
    ) {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            state.layers.forEach { layer ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Checkbox(
                        checked = layer.visible,
                        onCheckedChange = { state.showLayer(layer.at, it) },
                    )
                    Text(
                        layer.name,
                        style = MaterialTheme.typography.bodySmall,
                        color = Color(0xFFDDE3EA),
                        maxLines = 1,
                    )
                }
            }
        }
    }
}

/** The picture the ribbon takes, and getting it out of the app. */
@Composable
private fun Captures(state: DrawingViewerState) {
    val context = LocalContext.current

    // **The reader's own editor, not a second one.** Every tool it offers on a
    // page — pen, line, arrow, box, circle, cloud, highlighter, eraser, typed
    // captions with their font, size and bend — works the same way on a picture
    // of a drawing, because a picture is a picture. What it needed was somewhere
    // to keep the marks that is not a document, and something to burn them into
    // that is not a page re-render.
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
            // A picture of a drawing is all drawing: there is no bare area
            // around a page for a fill to reach, so the control hides itself.
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
            // A drawing has no pages, so a page number here would be a fact
            // this editor invented.
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
        color = Color(0xFFE7A94B),
        textAlign = TextAlign.Center,
        modifier = Modifier.padding(32.dp),
    )
}

/** The same ground the model viewer uses, so the two feel like one app. */
private val BACKDROP = Color(0xFF1E2024)

/**
 * How far a finger may move and still be a tap, in pixels.
 *
 * A press on a phone is never perfectly still. Too small and a measurement can
 * only be taken by somebody with a steady hand; too large and a short pan
 * quietly places a point where they were trying to slide from.
 */
private const val STILL = 12f

/**
 * The measurement, as a chip above the ribbon.
 *
 * Says what it snapped to as well as how far it is. Somebody measuring a wall
 * needs to know the number came from its endpoint and not from a spot near it,
 * and there is no other way to tell once the answer is on screen.
 */
@Composable
private fun MeasureReadout(taken: Measurement, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = Color(0xF2262A31),
        shadowElevation = 6.dp,
    ) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 10.dp)) {
            Text(
                taken.readout(),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                color = Color(0xFFFFBE3C),
            )
            if (taken.taken == 2) {
                Text(
                    taken.snaps.joinToString(" → "),
                    style = MaterialTheme.typography.bodySmall,
                    color = Color(0xFFB8BEC6),
                )
            }
        }
    }
}
