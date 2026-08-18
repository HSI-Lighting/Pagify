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
import androidx.compose.ui.draw.drawWithContent
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PenMode
import com.hsilighting.pagify.core.TextSegment

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
    onAdd: (Annotation) -> Unit,
    onRequestNote: (Offset) -> Unit,
): Modifier {
    // Read through `rememberUpdatedState`: the pointerInput block below is keyed
    // on the tool, so without this it would capture the colour and mode that were
    // current when the gesture handler started rather than the latest ones.
    val currentColor by rememberUpdatedState(penColor)
    val currentMode by rememberUpdatedState(penMode)
    val currentSegments by rememberUpdatedState(textSegments)
    val add by rememberUpdatedState(onAdd)
    val requestNote by rememberUpdatedState(onRequestNote)
    val scale by rememberUpdatedState(renderScale)

    /** Live stroke, in page points, while a marker drag is in progress. */
    var wetStroke by remember { mutableStateOf<List<Offset>>(emptyList()) }
    /** Live highlight rects while a highlight drag is in progress. */
    var wetHighlight by remember { mutableStateOf<List<Rect>>(emptyList()) }

    fun toPage(position: Offset): Offset =
        if (scale > 0f) Offset(position.x / scale, position.y / scale) else Offset.Zero

    /** Lines the drag has swept, clipped to the horizontal extent actually covered. */
    fun highlightFor(startPage: Offset, endPage: Offset): List<Rect> {
        val hits = currentSegments.filter { it.intersectsBand(startPage.y, endPage.y) }
        if (hits.isEmpty()) return emptyList()

        val first = minOf(startPage.y, endPage.y)
        val last = maxOf(startPage.y, endPage.y)
        return hits.map { segment ->
            // Only the first and last lines are trimmed to where the drag began
            // and ended; the lines between are taken whole, which is what reading
            // a selection top-to-bottom means.
            val isFirst = segment.top <= first && segment.bottom >= first
            val isLast = segment.top <= last && segment.bottom >= last
            val left = if (isFirst && !isLast) maxOf(segment.left, minOf(startPage.x, endPage.x))
                else if (isFirst && isLast) maxOf(segment.left, minOf(startPage.x, endPage.x))
                else segment.left
            val right = if (isLast && !isFirst) minOf(segment.right, maxOf(startPage.x, endPage.x))
                else if (isFirst && isLast) minOf(segment.right, maxOf(startPage.x, endPage.x))
                else segment.right
            Rect(
                left = minOf(left, right),
                top = segment.top,
                right = maxOf(left, right),
                bottom = segment.bottom,
            )
        }.filter { it.width > 0.5f }
    }

    val inputModifier = when (tool) {
        AnnotationTool.Pen -> Modifier.pointerInput(pageIndex, tool) {
            var start = Offset.Zero
            detectDragGestures(
                onDragStart = { position ->
                    start = toPage(position)
                    wetStroke = listOf(start)
                    wetHighlight = emptyList()
                },
                onDrag = { change, _ ->
                    change.consume()
                    val here = toPage(change.position)
                    when (currentMode) {
                        PenMode.Marker -> wetStroke = wetStroke + here
                        PenMode.Highlight -> wetHighlight = highlightFor(start, here)
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
                            add(
                                Annotation.Highlight(
                                    id = 0L,
                                    pageIndex = pageIndex,
                                    rects = wetHighlight,
                                    color = currentColor,
                                ),
                            )
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
            detectTapGestures { position -> requestNote(toPage(position)) }
        }

        // Signature and Snapshot are driven from their own surfaces, and None must
        // leave every gesture to the reader.
        else -> Modifier
    }

    return this
        .then(inputModifier)
        .drawWithContent {
            drawContent()
            annotations.forEach { drawAnnotation(it, scale) }
            if (wetHighlight.isNotEmpty()) {
                drawHighlightRects(wetHighlight, currentColor, scale)
            }
            if (wetStroke.size > 1) {
                drawInkStroke(wetStroke, currentColor, MARKER_WIDTH_POINTS, scale)
            }
        }
}

private fun DrawScope.drawAnnotation(annotation: Annotation, scale: Float) {
    when (annotation) {
        is Annotation.Highlight -> drawHighlightRects(annotation.rects, annotation.color, scale)
        is Annotation.Ink ->
            drawInkStroke(annotation.points, annotation.color, annotation.strokeWidth, scale)
        is Annotation.Signature -> annotation.strokes.forEach { stroke ->
            drawInkStroke(stroke, annotation.color, SIGNATURE_WIDTH_POINTS, scale)
        }
        is Annotation.Note -> {
            // A small anchored marker; the note's text is shown in a sheet rather
            // than painted onto the page, where it would obscure what it annotates.
            val centre = Offset(annotation.anchor.x * scale, annotation.anchor.y * scale)
            drawCircle(Color(annotation.color), radius = NOTE_RADIUS_DP * scale, center = centre)
        }
    }
}

private fun DrawScope.drawHighlightRects(rects: List<Rect>, color: Long, scale: Float) {
    // Multiply, so the page's own text stays legible through the wash rather than
    // being covered by a flat translucent block.
    rects.forEach { r ->
        drawRect(
            color = Color(color).copy(alpha = HIGHLIGHT_ALPHA),
            topLeft = Offset(r.left * scale, r.top * scale),
            size = Size(r.width * scale, r.height * scale),
        )
    }
}

private fun DrawScope.drawInkStroke(
    points: List<Offset>,
    color: Long,
    widthPoints: Float,
    scale: Float,
) {
    if (points.size < 2) return
    val path = Path().apply {
        moveTo(points[0].x * scale, points[0].y * scale)
        // Quadratic midpoints, so a fast drag reads as a smooth line instead of
        // the visible polygon that joining raw touch samples produces.
        for (i in 1 until points.size) {
            val prev = points[i - 1]
            val cur = points[i]
            val midX = (prev.x + cur.x) / 2f * scale
            val midY = (prev.y + cur.y) / 2f * scale
            quadraticBezierTo(prev.x * scale, prev.y * scale, midX, midY)
        }
        val last = points.last()
        lineTo(last.x * scale, last.y * scale)
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
private const val NOTE_RADIUS_DP = 7f
