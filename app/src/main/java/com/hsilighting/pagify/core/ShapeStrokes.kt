package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.sin

/**
 * Turning a drag into the strokes a shape is made of, in page points.
 *
 * Every shape the reader draws — a line, an arrow, a box, a circle — is saved
 * into the PDF as **ink**: a set of polylines, which is what a signature already
 * is and what every viewer on earth draws correctly. A true `Square` annotation
 * would be truer to the spec, but PDFium only generates appearances for some
 * subtypes, so lines and arrows would fall back to ink anyway and the app would
 * carry two mechanisms where one will do.
 *
 * The line type is **baked in here**, as separate strokes with gaps between them,
 * for the same reason: a dash array on an ink annotation is a thing most viewers
 * ignore. What is drawn is what is in the file.
 *
 * Pure, and separate from the layer that captures the drag, because this is the
 * part that is wrong in ways that look plausible — an arrow head at the wrong end,
 * an ellipse that is a rectangle's inscribed circle rather than its own.
 */
fun shapeStrokes(
    tool: AnnotationTool,
    start: Offset,
    end: Offset,
    style: MarkupStyle,
    widthPoints: Float,
): List<List<Offset>> {
    val outline = when (tool) {
        AnnotationTool.Line -> listOf(listOf(start, end))
        AnnotationTool.Arrow -> arrowStrokes(start, end, widthPoints)
        AnnotationTool.Rectangle -> listOf(rectangleOutline(start, end))
        AnnotationTool.Ellipse -> listOf(ellipseOutline(start, end))
        else -> return emptyList()
    }

    return outline.flatMap { dashed(it, style, widthPoints) }
}

/**
 * The shaft and the two barbs.
 *
 * Three strokes rather than one, so the head keeps its point: a single polyline
 * running out to one barb and back would round off at the tip, and the tip is the
 * part an arrow is for.
 *
 * The head is left solid by the caller — see [shapeStrokes], which dashes only
 * what it is given. A barb is a few widths long, so a dash pattern lands on it as
 * a stub or misses it entirely.
 */
private fun arrowStrokes(start: Offset, end: Offset, widthPoints: Float): List<List<Offset>> {
    val angle = atan2(end.y - start.y, end.x - start.x)
    val head = widthPoints * ARROW_HEAD_WIDTHS

    return listOf(listOf(start, end)) + listOf(-1f, 1f).map { side ->
        val barb = angle + PI.toFloat() + side * ARROW_HEAD_ANGLE
        listOf(end, Offset(end.x + head * cos(barb), end.y + head * sin(barb)))
    }
}

/** A closed rectangle from two dragged corners, whichever way round they came. */
private fun rectangleOutline(start: Offset, end: Offset): List<Offset> {
    val left = minOf(start.x, end.x)
    val right = maxOf(start.x, end.x)
    val top = minOf(start.y, end.y)
    val bottom = maxOf(start.y, end.y)

    return listOf(
        Offset(left, top),
        Offset(right, top),
        Offset(right, bottom),
        Offset(left, bottom),
        Offset(left, top),
    )
}

/**
 * An ellipse inscribed in the dragged rectangle, as a closed polyline.
 *
 * Sampled rather than drawn with curves because ink has no curves: a PDF ink
 * annotation is a list of points, so the smoothness has to be in the sampling.
 * [ELLIPSE_SEGMENTS] is enough that the joins are invisible at any zoom a reader
 * offers, and small enough that the annotation stays a reasonable size.
 */
private fun ellipseOutline(start: Offset, end: Offset): List<Offset> {
    val centreX = (start.x + end.x) / 2f
    val centreY = (start.y + end.y) / 2f
    val radiusX = kotlin.math.abs(end.x - start.x) / 2f
    val radiusY = kotlin.math.abs(end.y - start.y) / 2f

    // Closed by repeating the first point rather than by walking a full turn:
    // cos(2pi) is not exactly cos(0), and the float that separates them leaves a
    // hairline gap in the stroke where the ellipse joins itself.
    val ring = (0 until ELLIPSE_SEGMENTS).map { step ->
        val angle = step.toFloat() / ELLIPSE_SEGMENTS * 2f * PI.toFloat()
        Offset(centreX + radiusX * cos(angle), centreY + radiusY * sin(angle))
    }

    return ring + ring.first()
}

/**
 * Cut a polyline into the dashes its line type asks for.
 *
 * Walks the line by length rather than by point, so a dash falls where it should
 * on a curve as well as on a straight run — an ellipse is a hundred short
 * segments, and dashing per segment would put a dash on each one.
 *
 * A solid line comes back as itself, in one piece: the common case pays nothing.
 */
fun dashed(points: List<Offset>, style: MarkupStyle, widthPoints: Float): List<List<Offset>> {
    val pattern = style.dashPattern(widthPoints)
    if (pattern.isEmpty() || points.size < 2) return listOf(points)

    val strokes = mutableListOf<List<Offset>>()
    var current = mutableListOf(points.first())

    var index = 0
    var drawing = true
    var left = pattern[0]

    for (segment in points.zipWithNext()) {
        var from = segment.first
        val to = segment.second
        var remaining = (to - from).getDistance()

        while (remaining > left) {
            val at = from + (to - from) * (left / remaining)
            if (drawing) {
                current += at
                strokes += current.toList()
                current = mutableListOf()
            } else {
                current = mutableListOf(at)
            }

            from = at
            remaining -= left
            index = (index + 1) % pattern.size
            left = pattern[index]
            drawing = !drawing
        }

        left -= remaining
        if (drawing) current += to
    }

    if (drawing && current.size > 1) strokes += current.toList()
    return strokes
}

/**
 * The dash pattern in page points, or empty for a solid line.
 *
 * The same proportions the capture editor and the engine draw with, so a dashed
 * line on a page and a dashed line on a picture of that page are the same line.
 * A dot is a segment of almost no length; ink has round caps, so it draws as a
 * nib rather than as nothing.
 */
fun MarkupStyle.dashPattern(widthPoints: Float): List<Float> {
    val w = widthPoints.coerceAtLeast(0.5f)
    val dot = w * DOT_LENGTH
    return when (this) {
        MarkupStyle.SOLID -> emptyList()
        MarkupStyle.DASH_1 -> listOf(w * 4f, w * 3f)
        MarkupStyle.DASH_2 -> listOf(w * 9f, w * 4f)
        MarkupStyle.CENTERLINE_1 -> listOf(w * 9f, w * 3f, dot, w * 3f)
        MarkupStyle.CENTERLINE_2 -> listOf(w * 9f, w * 3f, dot, w * 3f, dot, w * 3f)
    }
}

/** How long a dot is, as a fraction of the stroke width. */
private const val DOT_LENGTH = 0.35f

/** Arrow head length, as a multiple of the stroke width. */
private const val ARROW_HEAD_WIDTHS = 4f

/** Half-angle of the arrow head, in radians — about 25°. */
private const val ARROW_HEAD_ANGLE = 0.44f

/** How many segments an ellipse is sampled into. */
private const val ELLIPSE_SEGMENTS = 96
