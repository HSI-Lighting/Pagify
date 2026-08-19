package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.draw.drawWithContent
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.NOTE_MARKER_RADIUS_POINTS
import com.hsilighting.pagify.core.isHitBy
import com.hsilighting.pagify.core.PenMode
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.core.TextSelection

/**
 * Draws a page's annotations and, when a tool is active, captures the input that
 * creates new ones.
 *
 * Everything here works in **page points**, converting to pixels only at draw
 * time via [renderScale]. Storing a stroke in screen pixels would freeze it to
 * the zoom level it was drawn at, and it would drift the moment the page was
 * re-rendered at another size.
 *
 * When no tool is selected this adds no pointer input at all, so scrolling and
 * zooming behave exactly as before — an always-on input layer would swallow the
 * gestures the reader depends on.
 */
@Composable
fun Modifier.annotationLayer(
    pageIndex: Int,
    annotations: List<Annotation>,
    textSegments: List<TextSegment>,
    tool: AnnotationTool,
    penMode: PenMode,
    penColor: Long,
    renderScale: Float,
    /**
     * Where the page's top-left corner sits inside this element, in pixels.
     *
     * Zero in the list, where the layer is applied straight to the page. The
     * magnified view draws a translated page into a viewport-sized canvas, so
     * without this the layer would map page points as though the page still
     * began at the corner — marks would land at the wrong place and a touch
     * would be read as pointing somewhere else entirely.
     */
    contentOffset: Offset = Offset.Zero,
    onAdd: (Annotation) -> Unit,
    onRequestNote: (Offset) -> Unit,
    /** A note marker was tapped; show what it says. */
    onOpenNote: (Annotation.Note) -> Unit = {},
    /** Opens an eraser stroke; everything it takes until [onEraseEnd] is one undo. */
    onEraseStart: () -> Unit = {},
    /** Rub out whatever is at this page point, within this tolerance in points. */
    onErase: (point: Offset, tolerancePoints: Float) -> Unit = { _, _ -> },
    onEraseEnd: () -> Unit = {},
    /**
     * A highlight drag that swept the page and selected nothing.
     *
     * The honest signal that the highlighter cannot work here, and better than
     * counting runs: a scan often carries one stray run — a watermark, a page
     * number — so "no text at all" misses exactly the pages that look like they
     * should work and do not.
     */
    onHighlightMissed: () -> Unit = {},
): Modifier {
    // Read through `rememberUpdatedState`: the pointerInput block below is keyed
    // on the tool, so without this it would capture the colour and mode that were
    // current when the gesture handler started rather than the latest ones.
    val currentColor by rememberUpdatedState(penColor)
    val currentMode by rememberUpdatedState(penMode)
    val currentSegments by rememberUpdatedState(textSegments)
    val add by rememberUpdatedState(onAdd)
    val requestNote by rememberUpdatedState(onRequestNote)
    val openNote by rememberUpdatedState(onOpenNote)
    val currentAnnotations by rememberUpdatedState(annotations)
    val scale by rememberUpdatedState(renderScale)
    val origin by rememberUpdatedState(contentOffset)
    val eraseStart by rememberUpdatedState(onEraseStart)
    val erase by rememberUpdatedState(onErase)
    val eraseEnd by rememberUpdatedState(onEraseEnd)
    val highlightMissed by rememberUpdatedState(onHighlightMissed)

    /** Live stroke, in page points, while a marker drag is in progress. */
    var wetStroke by remember { mutableStateOf<List<Offset>>(emptyList()) }
    /** Live highlight rects while a highlight drag is in progress. */
    var wetHighlight by remember { mutableStateOf<List<Rect>>(emptyList()) }
    /** Where the eraser is, in page points, while it is down. Drawn as a ring. */
    var eraserAt by remember { mutableStateOf<Offset?>(null) }

    fun toPage(position: Offset): Offset =
        if (scale > 0f) (position - origin) / scale else Offset.Zero

    // A fixed touch radius in dp, converted to page points through the current
    // scale. Expressing the eraser's reach in points instead would make it a
    // pinhead when zoomed in and a paint roller when zoomed out.
    val eraserRadiusPx = with(LocalDensity.current) { ERASER_TOUCH_RADIUS.toPx() }
    fun tolerancePoints(): Float = if (scale > 0f) eraserRadiusPx / scale else 0f

    val inputModifier = when (tool) {
        AnnotationTool.Pen -> Modifier.pointerInput(pageIndex, tool) {
            var start = Offset.Zero
            /** Distinguishes a sweep that found nothing from a stray tap. */
            var swept = false
            detectDragGestures(
                onDragStart = { position ->
                    start = toPage(position)
                    swept = false
                    wetStroke = listOf(start)
                    wetHighlight = emptyList()
                },
                onDrag = { change, _ ->
                    change.consume()
                    swept = true
                    val here = toPage(change.position)
                    when (currentMode) {
                        PenMode.Marker -> wetStroke = wetStroke + here
                        PenMode.Highlight ->
                            wetHighlight = TextSelection.rectsBetween(currentSegments, start, here)
                    }
                },
                onDragEnd = {
                    when (currentMode) {
                        PenMode.Marker -> if (wetStroke.size > 1) {
                            add(
                                Annotation.Ink(
                                    id = 0L,
                                    pageIndex = pageIndex,
                                    points = wetStroke,
                                    color = currentColor,
                                    strokeWidth = MARKER_WIDTH_POINTS,
                                ),
                            )
                        }
                        PenMode.Highlight -> if (wetHighlight.isNotEmpty()) {
                            // The span the rects cover is recorded alongside their
                            // count, because a count on its own cannot distinguish
                            // "selected a paragraph" from "selected the page" —
                            // which is what the previous selection actually did.
                            SessionRecorder.record(
                                kind = "HIGHLIGHT_SELECT",
                                detail = "page=$pageIndex rects=${wetHighlight.size} " +
                                    "x=${wetHighlight.minOf { it.left }.toInt()}.." +
                                    "${wetHighlight.maxOf { it.right }.toInt()} " +
                                    "y=${wetHighlight.minOf { it.top }.toInt()}.." +
                                    "${wetHighlight.maxOf { it.bottom }.toInt()} " +
                                    "runsOnPage=${currentSegments.size} zoomed=${origin != Offset.Zero}",
                            )
                            add(
                                Annotation.Highlight(
                                    id = 0L,
                                    pageIndex = pageIndex,
                                    rects = wetHighlight,
                                    color = currentColor,
                                ),
                            )
                        } else if (swept) {
                            // Dragged across the page and selected nothing. On a
                            // scan that is every drag, and without saying so the
                            // tool simply looks broken.
                            SessionRecorder.record(
                                kind = "HIGHLIGHT_MISSED",
                                detail = "page=$pageIndex runsOnPage=${currentSegments.size}",
                            )
                            highlightMissed()
                        }
                    }
                    wetStroke = emptyList()
                    wetHighlight = emptyList()
                },
                onDragCancel = {
                    wetStroke = emptyList()
                    wetHighlight = emptyList()
                },
            )
        }

        AnnotationTool.Note -> Modifier.pointerInput(pageIndex, tool) {
            detectTapGestures { position ->
                val at = toPage(position)
                // A tap on an existing marker opens it; anywhere else starts a new
                // one. Without this the text a note holds could be typed and never
                // read again — the marker was the only thing the page ever showed,
                // which made the tool look like it added a dot and nothing else.
                //
                // Topmost first: later marks draw over earlier ones, so the last
                // match is the one under the finger.
                val hit = currentAnnotations
                    .filterIsInstance<Annotation.Note>()
                    .lastOrNull { it.isHitBy(at, tolerancePoints()) }

                if (hit != null) openNote(hit) else requestNote(at)
            }
        }

        // A tap rubs out the mark it lands on; a drag sweeps across several. Both
        // are needed: a drag never fires for a tap, because it has to pass touch
        // slop first, and a tap on a single highlight is the common case.
        AnnotationTool.Eraser -> Modifier
            .pointerInput(pageIndex, tool) {
                detectTapGestures { position ->
                    eraseStart()
                    erase(toPage(position), tolerancePoints())
                    eraseEnd()
                }
            }
            .pointerInput(pageIndex, tool) {
                detectDragGestures(
                    onDragStart = { position ->
                        eraseStart()
                        eraserAt = toPage(position)
                        erase(toPage(position), tolerancePoints())
                    },
                    onDrag = { change, _ ->
                        change.consume()
                        val here = toPage(change.position)
                        eraserAt = here
                        erase(here, tolerancePoints())
                    },
                    onDragEnd = {
                        eraserAt = null
                        eraseEnd()
                    },
                    onDragCancel = {
                        eraserAt = null
                        eraseEnd()
                    },
                )
            }

        // Signature and Snapshot are driven from their own surfaces, and None must
        // leave every gesture to the reader.
        else -> Modifier
    }

    return this
        .then(inputModifier)
        .drawWithContent {
            drawContent()
            annotations.forEach { drawAnnotation(it, scale, origin) }
            if (wetHighlight.isNotEmpty()) {
                drawHighlightRects(wetHighlight, currentColor, scale, origin)
            }
            if (wetStroke.size > 1) {
                drawInkStroke(wetStroke, currentColor, MARKER_WIDTH_POINTS, scale, origin)
            }
            eraserAt?.let { at ->
                // Shows exactly how far the eraser reaches, so a miss reads as a
                // miss rather than as the tool not working.
                drawCircle(
                    color = Color.Black.copy(alpha = 0.35f),
                    radius = eraserRadiusPx,
                    center = at * scale + origin,
                    style = Stroke(width = ERASER_RING_PX),
                )
            }
        }
}

private fun DrawScope.drawAnnotation(annotation: Annotation, scale: Float, origin: Offset) {
    when (annotation) {
        is Annotation.Highlight ->
            drawHighlightRects(annotation.rects, annotation.color, scale, origin)
        is Annotation.Ink ->
            drawInkStroke(annotation.points, annotation.color, annotation.strokeWidth, scale, origin)
        is Annotation.Signature -> annotation.strokes.forEach { stroke ->
            drawInkStroke(stroke, annotation.color, SIGNATURE_WIDTH_POINTS, scale, origin)
        }
        is Annotation.Note -> {
            // An anchored marker; the note's text is shown in a sheet rather than
            // painted onto the page, where it would obscure what it annotates.
            //
            // Outlined, rather than a plain disc in the pen colour. A 7 pt yellow
            // dot on white paper is genuinely hard to find, and the first version
            // of this was reported as the note not being added at all — which is
            // exactly what an invisible marker looks like.
            val centre = annotation.anchor * scale + origin
            val radius = NOTE_MARKER_RADIUS_POINTS * scale
            drawCircle(Color(annotation.color), radius = radius, center = centre)
            drawCircle(
                Color.Black.copy(alpha = NOTE_OUTLINE_ALPHA),
                radius = radius,
                center = centre,
                style = Stroke(width = NOTE_OUTLINE_WIDTH_POINTS * scale),
            )
            // A pip in the middle, so it reads as a marker rather than as a stray
            // blob of highlighter.
            drawCircle(
                Color.Black.copy(alpha = NOTE_OUTLINE_ALPHA),
                radius = radius * NOTE_PIP_FRACTION,
                center = centre,
            )
        }
    }
}

private fun DrawScope.drawHighlightRects(
    rects: List<Rect>,
    color: Long,
    scale: Float,
    origin: Offset,
) {
    // Multiply, so the page's own text stays legible through the wash rather than
    // being covered by a flat translucent block.
    rects.forEach { r ->
        drawRect(
            color = Color(color).copy(alpha = HIGHLIGHT_ALPHA),
            topLeft = Offset(r.left, r.top) * scale + origin,
            size = Size(r.width * scale, r.height * scale),
        )
    }
}

private fun DrawScope.drawInkStroke(
    points: List<Offset>,
    color: Long,
    widthPoints: Float,
    scale: Float,
    origin: Offset,
) {
    if (points.size < 2) return
    fun at(p: Offset) = p * scale + origin

    val path = Path().apply {
        val start = at(points[0])
        moveTo(start.x, start.y)
        // Quadratic midpoints, so a fast drag reads as a smooth line instead of
        // the visible polygon that joining raw touch samples produces.
        for (i in 1 until points.size) {
            val prev = at(points[i - 1])
            val cur = at(points[i])
            quadraticBezierTo(prev.x, prev.y, (prev.x + cur.x) / 2f, (prev.y + cur.y) / 2f)
        }
        val last = at(points.last())
        lineTo(last.x, last.y)
    }
    drawPath(
        path = path,
        color = Color(color),
        style = Stroke(
            width = widthPoints * scale,
            cap = StrokeCap.Round,
            join = StrokeJoin.Round,
        ),
    )
}

/** Highlighter wash. Low enough to read through, high enough to see. */
private const val HIGHLIGHT_ALPHA = 0.35f

/** Marker nib, in page points, so it thickens with the page rather than the screen. */
private const val MARKER_WIDTH_POINTS = 2.4f
private const val SIGNATURE_WIDTH_POINTS = 1.8f
private const val ERASER_RING_PX = 2f

/**
 * How far the eraser reaches from the touch point.
 *
 * A highlight is a few points tall and an ink stroke has no area at all, so an
 * exact hit test would ask for precision no finger has. Roughly half a fingertip.
 */
private val ERASER_TOUCH_RADIUS = 14.dp

/** Ink of the note marker's outline and pip. */
private const val NOTE_OUTLINE_ALPHA = 0.65f

/** Outline thickness, in page points, so it holds up at any zoom. */
private const val NOTE_OUTLINE_WIDTH_POINTS = 1.2f

/** Pip radius as a fraction of the marker's. */
private const val NOTE_PIP_FRACTION = 0.32f
