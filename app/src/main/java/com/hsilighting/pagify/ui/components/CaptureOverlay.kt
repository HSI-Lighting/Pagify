package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
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
import androidx.compose.ui.graphics.ClipOp
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.input.pointer.pointerInput
import com.hsilighting.pagify.core.isWorthCapturing
import com.hsilighting.pagify.core.lassoBounds
import com.hsilighting.pagify.core.rectFromCorners

/**
 * Drag around what to capture.
 *
 * Deliberately **over the whole reader** rather than inside a page. A capture is
 * whatever was framed on screen, and the reader stacks pages in a column, so the
 * interesting box very often takes the bottom of one page, the gap below it and
 * the top of the next. Living inside one page's layer made that impossible to
 * express — the drag was clipped to the page it started on — and it also put the
 * gesture behind the scroll container, which sometimes took it instead.
 *
 * Two shapes, one gesture. A box is the default. With [lasso] the same drag
 * traces a ring instead, for the thing a box cannot say: a detail on a busy
 * drawing with a title block beside it, one fitting out of a schedule. The
 * picture is still the ring's bounding box, because an image is a rectangle —
 * what the ring buys is that everything outside it comes back blank.
 *
 * Both are reported in this element's own pixels; turning that into per-page
 * tiles and a mask is `captureTilesFor` and `captureMaskFor`, which are pure and
 * tested on the host.
 *
 * @param onCapture the region, and the ring that framed it — empty for a box.
 */
fun Modifier.captureOverlay(
    lasso: Boolean = false,
    onCapture: (Rect, List<Offset>) -> Unit,
): Modifier = composed {
    val capture by rememberUpdatedState(onCapture)
    var box by remember { mutableStateOf<Rect?>(null) }

    // The ring is appended to in place and redrawn off a counter, rather than
    // being a new list every frame. A slow, careful drag around a detail is
    // hundreds of samples, and copying the list per sample is quadratic in
    // exactly the case the tool exists for.
    val ring = remember { mutableListOf<Offset>() }
    var ringRevision by remember { mutableIntStateOf(0) }

    this
        // Keyed on the shape: the two read the same gesture differently, and a
        // handler left over from the other one would keep dragging boxes after
        // the ring was chosen.
        .pointerInput(lasso) {
            var start = Offset.Zero
            detectDragGestures(
                onDragStart = { position ->
                    start = position
                    box = null
                    ring.clear()
                    if (lasso) ring += position
                    ringRevision++
                },
                onDrag = { change, _ ->
                    change.consume()
                    if (lasso) {
                        // Thinned as it goes: touch reports far more samples than
                        // the shape needs, and every one of them is a point the
                        // engine has to fill a path with later.
                        val last = ring.lastOrNull()
                        if (last == null ||
                            (change.position - last).getDistance() >= RING_STEP_PX
                        ) {
                            ring += change.position
                            ringRevision++
                        }
                    } else {
                        box = rectFromCorners(
                            start.x,
                            start.y,
                            change.position.x,
                            change.position.y,
                        )
                    }
                },
                onDragEnd = {
                    if (lasso) {
                        // `lassoBounds` decides whether it encloses anything; a
                        // ring too small to mean it is a tap that wandered.
                        val outline = ring.toList()
                        lassoBounds(outline)?.let { capture(it, outline) }
                    } else {
                        // A drag too small to mean it is a tap that moved.
                        // Capturing it would put a postage stamp and an editor in
                        // front of someone who was trying to scroll.
                        box?.takeIf { it.isWorthCapturing() }?.let { capture(it, emptyList()) }
                    }
                    box = null
                    ring.clear()
                    ringRevision++
                },
                onDragCancel = {
                    box = null
                    ring.clear()
                    ringRevision++
                },
            )
        }
        .drawWithContent {
            drawContent()
            // The revision is read so the draw is invalidated as the ring grows:
            // the ring itself is a plain list and cannot invalidate anything.
            if (ringRevision >= 0 && ring.size > 1) {
                drawRing(ring)
            } else {
                box?.let { drawMarquee(it) }
            }
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

/**
 * The ring, and the same promise the marquee makes: what is dimmed is what will
 * not be in the picture.
 *
 * Drawn closed from the first samples onwards, because closed is what it will be
 * — the ring is closed for the user rather than asking them to land their finger
 * back on the pixel they started from.
 */
private fun DrawScope.drawRing(points: List<Offset>) {
    val path = Path().apply {
        moveTo(points[0].x, points[0].y)
        for (index in 1 until points.size) lineTo(points[index].x, points[index].y)
        close()
    }

    clipPath(path, clipOp = ClipOp.Difference) {
        drawRect(Color.Black.copy(alpha = SHADE_ALPHA), size = size)
    }
    drawPath(path, Color.White, style = Stroke(width = BORDER_PX))
    drawPath(path, Color.Black.copy(alpha = 0.55f), style = Stroke(width = BORDER_PX / 2f))
}

/** How far the page outside a capture selection is knocked back. */
private const val SHADE_ALPHA = 0.45f

/** Marquee border, in pixels: a screen-space affordance, not part of the page. */
private const val BORDER_PX = 3f

/**
 * The closest two ring samples that are kept, in pixels.
 *
 * Small enough that a curve still reads as a curve, large enough that a slow drag
 * around a detail does not hand the engine thousands of points to fill a path
 * with.
 */
private const val RING_STEP_PX = 3f
