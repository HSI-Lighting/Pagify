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
import androidx.compose.ui.graphics.PathEffect
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
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import com.hsilighting.pagify.core.TextFrame
import com.hsilighting.pagify.core.layOutBlock
import com.hsilighting.pagify.core.textFrameBounds
import com.hsilighting.pagify.core.textFrameOutline
import com.hsilighting.pagify.core.widthOf
import com.hsilighting.pagify.core.TEXT_FRAME_STROKE
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.drag
import com.hsilighting.pagify.core.TEXT_GRAB_POINTS
import com.hsilighting.pagify.core.isHitBy
import com.hsilighting.pagify.core.movedBy
import com.hsilighting.pagify.core.scaledBy
import com.hsilighting.pagify.core.writesText
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
    /** A baseline for words has been placed; ask for them. */
    onPlaceText: (List<Offset>) -> Unit = {},
    /** Words already on the picture have been dragged somewhere else. */
    onMoveText: (index: Int, delta: Offset) -> Unit = { _, _ -> },
    /** A caption was tapped, so the ribbon's controls now belong to it. */
    onSelectText: (index: Int) -> Unit = {},
    /** The caption the ribbon is editing, drawn picked out. */
    selectedText: Int? = null,
    /** A caption was double-tapped; rewrite its words. */
    onEditText: (index: Int) -> Unit = {},
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
                onPlaceText = onPlaceText,
                onMoveText = onMoveText,
                onSelectText = onSelectText,
                selectedText = selectedText,
                onEditText = onEditText,
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
    /** A baseline for words has been placed; ask for them. */
    onPlaceText: (List<Offset>) -> Unit = {},
    /** Words already on the picture have been dragged somewhere else. */
    onMoveText: (index: Int, delta: Offset) -> Unit = { _, _ -> },
    /** A caption was tapped, so the ribbon's controls now belong to it. */
    onSelectText: (index: Int) -> Unit = {},
    /** The caption the ribbon is editing, drawn picked out. */
    selectedText: Int? = null,
    /** A caption was double-tapped; rewrite its words. */
    onEditText: (index: Int) -> Unit = {},
) {
    val commit by rememberUpdatedState(onCommit)
    val recognise by rememberUpdatedState(onRecognise)
    val placeText by rememberUpdatedState(onPlaceText)
    val moveText by rememberUpdatedState(onMoveText)
    val selectText by rememberUpdatedState(onSelectText)
    // Read live, not captured: the gesture block is keyed on the tool, so a plain
    // capture would hold whatever was selected when the tool was picked up.
    val heldCaption by rememberUpdatedState(selectedText)
    val editText by rememberUpdatedState(onEditText)
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

    /** When the last tap on a caption lifted, and which one, for double taps. */
    var lastTapAt by remember(tool) { mutableStateOf(0L) }
    var lastTapIndex by remember(tool) { mutableStateOf(-1) }

    /** Which mark is being dragged, and how far it has come. */
    var movingIndex by remember(tool) { mutableStateOf(-1) }
    var moveShift by remember(tool) { mutableStateOf(Offset.Zero) }


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
                } else if (tool.writesText) {
                    // Text is not dragged out, so it does not go through the
                    // gesture machine at all: a tap places a baseline and the
                    // words are asked for afterwards, a drag on words already
                    // there moves them, and the curved one traces its line.
                    Modifier.pointerInput(tool, crop, widthPx, heightPx) {
                        awaitEachGesture {
                            val down = awaitFirstDown(requireUnconsumed = false)
                            down.consume()
                            val start = toPage(down.position)

                            val grab = marks.indexOfLast { mark ->
                                val shape = mark.shape
                                shape is MarkupShape.Text && shape.isHitBy(start, TEXT_GRAB_POINTS)
                            }
                            if (grab < 0) {
                                // Nothing under the finger, so this is a tap that
                                // places words. Claimed here rather than left to
                                // a tap detector: the editor's own pinch handler
                                // consumes the press first, and a tap detector
                                // that asks for an unclaimed one never fires — so
                                // words tapped onto the picture never appeared.
                                var strayed = false
                                while (true) {
                                    val change = awaitPointerEvent().changes.first()
                                    change.consume()
                                    if (!change.pressed) break
                                    val travelled = (change.position - down.position).getDistance()
                                    if (travelled > viewConfiguration.touchSlop) strayed = true
                                }
                                // A drag that started on nothing is a drag, not a
                                // tap, and placing words at the end of one would
                                // be a mark nobody asked for.
                                if (!strayed) {
                                    if (heldCaption != null) {
                                        // A caption in hand: put it down. That is
                                        // also how the picture gets its pinch
                                        // back, since a held caption takes it.
                                        selectText(-1)
                                    } else {
                                        placeText(listOf(start))
                                    }
                                }
                                return@awaitEachGesture
                            }

                            var last = start
                            var shifted = Offset.Zero
                            drag(down.id) { change ->
                                change.consume()
                                val now = toPage(change.position)
                                shifted += now - last
                                last = now
                                movingIndex = grab
                                moveShift = shifted
                            }

                            // A press that went nowhere is a tap, and a tap on
                            // words picks them up for the ribbon to work on. A
                            // second one soon after opens them for rewriting —
                            // detected here rather than by a tap detector above,
                            // because this handler claims the press first and one
                            // asking for an unclaimed press would never fire.
                            if (movingIndex >= 0) {
                                moveText(grab, shifted)
                                lastTapAt = 0L
                            } else {
                                val now = System.currentTimeMillis()
                                val quick = now - lastTapAt < viewConfiguration.doubleTapTimeoutMillis
                                if (quick && lastTapIndex == grab) {
                                    editText(grab)
                                    lastTapAt = 0L
                                } else {
                                    selectText(grab)
                                    lastTapAt = now
                                    lastTapIndex = grab
                                }
                            }
                            movingIndex = -1
                            moveShift = Offset.Zero
                        }
                    }
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
            marks.forEachIndexed { index, it ->
                val shape = it.shape
                val shown = if (shape is MarkupShape.Text && index == movingIndex) {
                    shape.movedBy(moveShift)
                } else {
                    shape
                }
                drawMarkup(shown, it.color, it.widthPoints * scale, it.style, ::toPixels, scale)
                if (shown is MarkupShape.Text && index == selectedText) {
                    drawCaptionSelection(shown, ::toPixels, scale)
                }
            }
            // The wet stroke is drawn through the same builder the commit uses, so
            // what is under the finger is what ends up in the file — including the
            // highlighter's intensity, which rides in the colour's alpha.
            preview.forEach { shape ->
                val wet = markupFor(shape, currentTool, ink, currentSize, currentStyle)
                drawMarkup(wet.shape, wet.color, wet.widthPoints * scale, wet.style, ::toPixels, scale)
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
    /** Capture units to pixels, for the one mark whose size is not a stroke width. */
    scale: Float,
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
        is MarkupShape.Text -> drawMarkupText(shape, ink, toPixels, scale)
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

/**
 * Words on the picture, drawn glyph by glyph.
 *
 * Through the platform's own text drawing rather than the outlines the export
 * flattens them to: on screen this has to be sharp at any zoom, and the outlines
 * exist only because a file cannot hold text. Both walk the same layout, so what
 * is on screen is where the letters land.
 */
private fun DrawScope.drawMarkupText(
    shape: MarkupShape.Text,
    ink: Color,
    toPixels: (Offset) -> Offset,
    scale: Float,
) {
    if (shape.text.isBlank() || shape.path.isEmpty()) return

    // The frame first, so the words sit over it where they meet.
    if (shape.frame != TextFrame.None) {
        val box = textFrameBounds(
            anchor = shape.path.first(),
            runWidth = shape.font.widthOf(shape.text, shape.sizePoints),
            sizePoints = shape.sizePoints,
        )
        val ring = textFrameOutline(box, shape.sizePoints, shape.frame)
        if (ring.size >= 2) {
            val path = Path().apply {
                val start = toPixels(ring.first())
                moveTo(start.x, start.y)
                ring.drop(1).forEach { point ->
                    val at = toPixels(point)
                    lineTo(at.x, at.y)
                }
            }
            drawPath(
                path = path,
                color = ink,
                style = Stroke(
                    width = (shape.sizePoints * TEXT_FRAME_STROKE * scale).coerceAtLeast(1f),
                    cap = StrokeCap.Round,
                    join = StrokeJoin.Round,
                ),
            )
        }
    }

    val paint = android.graphics.Paint().apply {
        isAntiAlias = true
        color = ink.toArgb()
        textSize = shape.sizePoints * scale
        typeface = android.graphics.Typeface.create(
            shape.font.family,
            if (shape.font.bold) android.graphics.Typeface.BOLD else android.graphics.Typeface.NORMAL,
        )
    }

    drawIntoCanvas { canvas ->
        shape.layOutBlock().forEach { placement ->
            val at = toPixels(placement.origin)
            val native = canvas.nativeCanvas
            val saved = native.save()
            native.rotate(
                Math.toDegrees(placement.radians.toDouble()).toFloat(),
                at.x,
                at.y,
            )
            native.drawText(placement.character.toString(), at.x, at.y, paint)
            native.restoreToCount(saved)
        }
    }
}

/**
 * The caption the ribbon is editing, picked out.
 *
 * The same dashed amber box the reader draws round a selected caption, for the
 * same reason: without it the controls are visibly about nothing.
 */
private fun DrawScope.drawCaptionSelection(
    shape: MarkupShape.Text,
    toPixels: (Offset) -> Offset,
    scale: Float,
) {
    val anchor = shape.path.firstOrNull() ?: return
    val box = textFrameBounds(
        anchor = anchor,
        runWidth = shape.font.widthOf(shape.text, shape.sizePoints),
        sizePoints = shape.sizePoints,
    ).inflate(CAPTION_SELECTION_GAP / scale.coerceAtLeast(0.01f))

    val corner = toPixels(box.topLeft)
    val far = toPixels(box.bottomRight)
    val stroke = (shape.sizePoints * CAPTION_SELECTION_STROKE * scale).coerceIn(1.5f, 6f)

    drawRect(
        color = CAPTION_SELECTION_INK,
        topLeft = corner,
        size = Size(far.x - corner.x, far.y - corner.y),
        style = Stroke(
            width = stroke,
            pathEffect = PathEffect.dashPathEffect(floatArrayOf(stroke * 3f, stroke * 2.5f)),
        ),
    )
}

/** How far the box stands off the words, in screen pixels at any zoom. */
private const val CAPTION_SELECTION_GAP = 6f

private const val CAPTION_SELECTION_STROKE = 0.09f

private val CAPTION_SELECTION_INK = Color(0xFFF2A93B)
