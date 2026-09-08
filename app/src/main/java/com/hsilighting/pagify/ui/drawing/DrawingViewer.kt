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
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import com.hsilighting.pagify.ui.components.CaptureHint
import com.hsilighting.pagify.ui.components.captureOverlay
import com.hsilighting.pagify.ui.model.CaptureSheet
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
                    else -> Sheet(state)
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
private fun Sheet(state: DrawingViewerState) {
    Box(
        Modifier
            .fillMaxSize()
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
                            state.pan(moved.x / size.width, moved.y / size.height)
                        }
                        touching.forEach { it.consume() }
                    }

                    state.endGesture()
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

    val storage = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) state.savePicture(context) else state.noteStorageRefused()
    }

    state.taken?.let { picture ->
        CaptureSheet(
            picture = picture,
            format = state.captureFormat,
            onFormat = state::chooseFormat,
            onSave = {
                if (CaptureExport.galleryNeedsPermission()) {
                    storage.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                } else {
                    state.savePicture(context)
                }
            },
            onShare = { state.sharePicture(context) },
            onCopy = { state.copyPicture(context) },
            onDismiss = state::discardCapture,
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
