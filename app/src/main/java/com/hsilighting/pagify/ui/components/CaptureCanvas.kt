package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.MARKUP_DWELL_MILLIS
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.MarkupGesture
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.markupFor
import kotlinx.coroutines.withTimeoutOrNull

/**
 * The capture, with its marks drawn over it and the input that adds more.
 *
 * The picture on screen is a downscaled preview; the marks are drawn on top of it
 * by Compose rather than composited into it, so a stroke follows the finger at
 * frame rate and no engine call happens while one is down. The exported file is
 * rendered separately, by the engine, from these same shapes — see roadmap
 * decision 4.7 for why the split is here.
 *
 * Everything crossing the boundary is in **page points**. The mapping is the
 * displayed rectangle onto the crop, so it holds however the preview happens to
 * be scaled or letterboxed.
 */
@Composable
fun CaptureCanvas(
    image: ImageBitmap,
    /** The picture's own bounds, in capture units. Marks are placed against it. */
    crop: Rect,
    markup: List<Markup>,
    tool: MarkupTool,
    /** Whether the tool is held. Nothing draws when it is not. */
    armed: Boolean,
    color: Long,
    /** Nib width, or the highlighter's intensity. */
    size: Float,
    style: MarkupStyle,
    onCommit: (MarkupShape) -> Unit,
    /** Held still before lifting: ask the engine what this stroke was. */
    onRecognise: (List<Offset>) -> Unit,
    /**
     * Magnification of the picture on screen. Nothing to do with the export
     * scale: this is for looking closely and drawing accurately, and the file is
     * rendered at its own resolution regardless.
     */
    zoom: Float = 1f,
    pan: Offset = Offset.Zero,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier, contentAlignment = Alignment.Center) {
        // Fit the picture inside the available space, then work in that rectangle.
        // Deriving the mapping from the *displayed* size rather than the preview's
        // pixel size keeps it right whatever the decoder chose to downsample to.
        val aspect = image.width.toFloat() / image.height.toFloat()
        val boxAspect = maxWidth / maxHeight
        val shownWidth = if (aspect >= boxAspect) maxWidth else maxHeight * aspect
        val shownHeight = if (aspect >= boxAspect) maxWidth / aspect else maxHeight

        val density = LocalDensity.current
        val widthPx = with(density) { shownWidth.toPx() }
        val heightPx = with(density) { shownHeight.toPx() }

        // The zoom is a layer transform, so the drawing surface underneath keeps
        // working in unzoomed coordinates: Compose maps touches back through the
        // same transform, which means the markup arithmetic needs no notion of
        // zoom at all. Doing it the other way — scaling the coordinates by hand —
        // is how a stroke ends up landing somewhere other than under the finger.
        Box(
            Modifier
                .size(shownWidth, shownHeight)
                .graphicsLayer {
                    scaleX = zoom
                    scaleY = zoom
                    translationX = pan.x
                    translationY = pan.y
                },
        ) {
            Image(
                bitmap = image,
                contentDescription = "The captured region",
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize(),
            )

            MarkupSurface(
                widthPx = widthPx,
                heightPx = heightPx,
                crop = crop,
                markup = markup,
                tool = tool,
                armed = armed,
                color = color,
                size = size,
                style = style,
                onCommit = onCommit,
                onRecognise = onRecognise,
            )
        }
    }
}

@Composable
private fun MarkupSurface(
    widthPx: Float,
    heightPx: Float,
    crop: Rect,
    markup: List<Markup>,
    tool: MarkupTool,
    armed: Boolean,
    color: Long,
    size: Float,
    /** The style the *next* mark will use; each committed mark carries its own. */
    style: MarkupStyle,
    onCommit: (MarkupShape) -> Unit,
    onRecognise: (List<Offset>) -> Unit,
) {
    val commit by rememberUpdatedState(onCommit)
    val recognise by rememberUpdatedState(onRecognise)
    val marks by rememberUpdatedState(markup)
    val ink by rememberUpdatedState(color)
    val currentTool by rememberUpdatedState(tool)
    val currentSize by rememberUpdatedState(size)
    val currentStyle by rememberUpdatedState(style)

    // Re-made when the tool changes: a gesture belongs to one tool, and carrying a
    // half-drawn stroke into another would commit it as the wrong shape. Also when
    // the size changes, because the cloud sizes its scallops from it and a gesture
    // holding the number from before would draw a cloud nobody asked for.
    val gesture = remember(tool, size) { MarkupGesture(tool, size) }
    var preview by remember(tool) { mutableStateOf<List<MarkupShape>>(emptyList()) }
    var dwelling by remember(tool) { mutableStateOf(false) }

    /** Displayed pixels to page points. */
    fun toPage(position: Offset) = Offset(
        crop.left + (position.x / widthPx) * crop.width,
        crop.top + (position.y / heightPx) * crop.height,
    )

    /** Page points back to displayed pixels, for drawing. */
    fun toPixels(point: Offset) = Offset(
        (point.x - crop.left) / crop.width * widthPx,
        (point.y - crop.top) / crop.height * heightPx,
    )

    val scale = if (crop.width > 0f) widthPx / crop.width else 1f

    Box(
        Modifier
            .fillMaxSize()
            // Clipped to the picture, because that is what a mark is on. A stroke
            // that wandered past the edge was painted straight over the toolbar
            // below — Compose does not clip a canvas to its own bounds — and the
            // part beyond the edge is not in the export either, so showing it was
            // a promise the file would not keep.
            .clipToBounds()
            // No tool held, no input at all — exactly as the reader does it. A
            // disabled handler that consumed the events and did nothing would
            // still swallow the pinch this exists to protect.
            .then(
                if (!armed) {
                    Modifier
                } else {
                    Modifier.pointerInput(tool, crop, widthPx, heightPx) {
                awaitPointerEventScope {
                    while (true) {
                        val down = awaitPointerEvent().changes.firstOrNull { it.pressed }
                            ?: continue
                        gesture.down(toPage(down.position))
                        down.consume()
                        preview = emptyList()
                        dwelling = false

                        var lifted = false
                        while (!lifted) {
                            // A still finger produces no events at all, so a
                            // timeout *is* the dwell — no polling, no timer, and
                            // nothing running between frames.
                            val event = withTimeoutOrNull(MARKUP_DWELL_MILLIS) {
                                awaitPointerEvent(PointerEventPass.Main)
                            }

                            if (event == null) {
                                gesture.still()
                                dwelling = gesture.isDwelling
                                continue
                            }

                            val change = event.changes.first()
                            change.consume()
                            if (!change.pressed) {
                                lifted = true
                            } else {
                                gesture.move(toPage(change.position))
                                dwelling = gesture.isDwelling
                                preview = gesture.preview
                            }
                        }

                        when (val outcome = gesture.up()) {
                            is MarkupGesture.Outcome.Commit ->
                                outcome.shapes.forEach { commit(it) }
                            is MarkupGesture.Outcome.Recognise -> recognise(outcome.points)
                            MarkupGesture.Outcome.Nothing -> Unit
                        }
                        preview = emptyList()
                        dwelling = false
                    }
                }
                    }
                },
            ),
    ) {
        androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
            marks.forEach {
                drawMarkup(it.shape, it.color, it.widthPoints * scale, it.style, ::toPixels)
            }
            // The wet stroke is drawn through the same builder the commit uses, so
            // what is under the finger is what ends up in the file — including the
            // highlighter's intensity, which rides in the colour's alpha.
            preview.forEach { shape ->
                val wet = markupFor(shape, currentTool, ink, currentSize, currentStyle)
                drawMarkup(wet.shape, wet.color, wet.widthPoints * scale, wet.style, ::toPixels)
            }
        }

        if (dwelling) {
            // Says the hold registered and what lifting will now do. Without it a
            // snap arrives unannounced, which is the one thing recognition must
            // never do.
            DwellHint(Modifier.align(Alignment.TopCenter))
        }
    }
}

@Composable
private fun DwellHint(modifier: Modifier) {
    androidx.compose.material3.Surface(
        modifier = modifier,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
        color = androidx.compose.material3.MaterialTheme.colorScheme.inverseSurface,
    ) {
        androidx.compose.material3.Text(
            text = "Release to snap to a shape",
            modifier = Modifier.size(width = 190.dp, height = 28.dp),
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            style = androidx.compose.material3.MaterialTheme.typography.labelMedium,
            color = androidx.compose.material3.MaterialTheme.colorScheme.inverseOnSurface,
        )
    }
}

private fun DrawScope.drawMarkup(
    shape: MarkupShape,
    color: Long,
    widthPx: Float,
    style: MarkupStyle,
    toPixels: (Offset) -> Offset,
) {
    val ink = Color(color)
    val effect = style.pathEffect(widthPx.coerceAtLeast(1f))
    val stroke = Stroke(
        width = widthPx.coerceAtLeast(1f),
        cap = StrokeCap.Round,
        join = StrokeJoin.Round,
        pathEffect = effect,
    )

    when (shape) {
        is MarkupShape.Freehand -> {
            if (shape.points.size < 2) return
            val path = Path().apply {
                val start = toPixels(shape.points.first())
                moveTo(start.x, start.y)
                for (i in 1 until shape.points.size) {
                    val previous = toPixels(shape.points[i - 1])
                    val current = toPixels(shape.points[i])
                    quadraticTo(
                        previous.x,
                        previous.y,
                        (previous.x + current.x) / 2f,
                        (previous.y + current.y) / 2f,
                    )
                }
                val last = toPixels(shape.points.last())
                lineTo(last.x, last.y)
            }
            drawPath(path, ink, style = stroke)
        }

        is MarkupShape.Line -> drawLine(
            color = ink,
            start = toPixels(shape.from),
            end = toPixels(shape.to),
            strokeWidth = stroke.width,
            pathEffect = effect,
            cap = StrokeCap.Round,
        )

        is MarkupShape.Arrow -> {
            val from = toPixels(shape.from)
            val to = toPixels(shape.to)
            drawLine(
                color = ink,
                start = from,
                end = to,
                strokeWidth = stroke.width,
                cap = StrokeCap.Round,
                pathEffect = effect,
            )

            val angle = kotlin.math.atan2(to.y - from.y, to.x - from.x)
            val head = stroke.width * ARROW_HEAD_LENGTHS
            for (side in listOf(-1f, 1f)) {
                val barb = angle + Math.PI.toFloat() + side * ARROW_HEAD_ANGLE
                drawLine(
                    color = ink,
                    start = to,
                    end = Offset(
                        to.x + head * kotlin.math.cos(barb),
                        to.y + head * kotlin.math.sin(barb),
                    ),
                    strokeWidth = stroke.width,
                    cap = StrokeCap.Round,
                )
            }
        }

        is MarkupShape.Rectangle -> {
            val (topLeft, size) = shape.rect.inPixels(toPixels)
            drawRect(ink, topLeft = topLeft, size = size, style = stroke)
        }

        is MarkupShape.Ellipse -> {
            val (topLeft, size) = shape.rect.inPixels(toPixels)
            drawOval(ink, topLeft = topLeft, size = size, style = stroke)
        }

        is MarkupShape.Highlight -> {
            val (topLeft, size) = shape.rect.inPixels(toPixels)
            // Filled and translucent, matching what the engine composites: a
            // highlight that covers what it marks has failed at its one job.
            // The alpha is the mark's own: the intensity slider sets it, and the
            // engine composites the export from the same number.
            drawRect(ink, topLeft = topLeft, size = size)
        }
    }
}

private fun Rect.inPixels(toPixels: (Offset) -> Offset): Pair<Offset, Size> {
    val topLeft = toPixels(Offset(left, top))
    val bottomRight = toPixels(Offset(right, bottom))
    return topLeft to Size(bottomRight.x - topLeft.x, bottomRight.y - topLeft.y)
}

/** Matches `render::markup` in the engine, so the preview is what gets exported. */
private const val ARROW_HEAD_LENGTHS = 4f
private const val ARROW_HEAD_ANGLE = 0.44f
