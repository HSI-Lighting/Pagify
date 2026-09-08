package com.hsilighting.pagify.ui.model

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CenterFocusStrong
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
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
            IconButton(onClick = state::fit) {
                Icon(
                    Icons.Filled.CenterFocusStrong,
                    contentDescription = "Fit the model to the view",
                    tint = Color.White,
                )
            }
        }

        Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
            when {
                state.error != null -> Message(state.error!!)
                state.loading -> CircularProgressIndicator(color = Color.White)
                else -> Surface(state)
            }
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
                            val moved = touching[0].positionChange()
                            if (abs(moved.x) > 0.01f || abs(moved.y) > 0.01f) {
                                state.orbit(moved.x / size.width, moved.y / size.height)
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
