package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import com.hsilighting.pagify.core.isWorthCapturing
import com.hsilighting.pagify.core.rectFromCorners

/**
 * Drag a box around what to capture.
 *
 * Deliberately **over the whole reader** rather than inside a page. A capture is
 * whatever was framed on screen, and the reader stacks pages in a column, so the
 * interesting box very often takes the bottom of one page, the gap below it and
 * the top of the next. Living inside one page's layer made that impossible to
 * express — the drag was clipped to the page it started on — and it also put the
 * gesture behind the scroll container, which sometimes took it instead.
 *
 * The rectangle it reports is in this element's own pixels; turning that into
 * per-page tiles is `captureTilesFor`, which is pure and tested on the host.
 */
fun Modifier.captureOverlay(onCapture: (Rect) -> Unit): Modifier = composed {
    val capture by rememberUpdatedState(onCapture)
    var box by remember { mutableStateOf<Rect?>(null) }

    this
        .pointerInput(Unit) {
            var start = Offset.Zero
            detectDragGestures(
                onDragStart = { position ->
                    start = position
                    box = null
                },
                onDrag = { change, _ ->
                    change.consume()
                    box = rectFromCorners(start.x, start.y, change.position.x, change.position.y)
                },
                onDragEnd = {
                    // A drag too small to mean it is a tap that moved. Capturing
                    // it would put a postage stamp and an editor in front of
                    // someone who was trying to scroll.
                    box?.takeIf { it.isWorthCapturing() }?.let(capture)
                    box = null
                },
                onDragCancel = { box = null },
            )
        }
        .drawWithContent {
            drawContent()
            box?.let { drawMarquee(it) }
        }
}

/**
 * The region a capture will take.
 *
 * Dimming everything outside it rather than only outlining it: the outline says
 * where the edges are, the dimming says what will be in the picture, and the
 * second is the question someone dragging this actually has.
 */
private fun DrawScope.drawMarquee(box: Rect) {
    val shade = Color.Black.copy(alpha = SHADE_ALPHA)
    val topLeft = Offset(box.left, box.top)
    val boxSize = Size(box.width, box.height)

    // Four bands around the selection: cheaper and more predictable than a clipped
    // full-size rectangle, which fights the layer's own clip.
    drawRect(shade, topLeft = Offset.Zero, size = Size(size.width, topLeft.y.coerceAtLeast(0f)))
    drawRect(
        shade,
        topLeft = Offset(0f, topLeft.y + boxSize.height),
        size = Size(size.width, (size.height - topLeft.y - boxSize.height).coerceAtLeast(0f)),
    )
    drawRect(
        shade,
        topLeft = Offset(0f, topLeft.y),
        size = Size(topLeft.x.coerceAtLeast(0f), boxSize.height),
    )
    drawRect(
        shade,
        topLeft = Offset(topLeft.x + boxSize.width, topLeft.y),
        size = Size((size.width - topLeft.x - boxSize.width).coerceAtLeast(0f), boxSize.height),
    )

    drawRect(Color.White, topLeft = topLeft, size = boxSize, style = Stroke(width = BORDER_PX))
    // A dark hairline inside the white one, so the edge is visible against both a
    // white page and a dark image.
    drawRect(
        Color.Black.copy(alpha = 0.55f),
        topLeft = topLeft,
        size = boxSize,
        style = Stroke(width = BORDER_PX / 2f),
    )
}

/** How far the page outside a capture selection is knocked back. */
private const val SHADE_ALPHA = 0.45f

/** Marquee border, in pixels: a screen-space affordance, not part of the page. */
private const val BORDER_PX = 3f
