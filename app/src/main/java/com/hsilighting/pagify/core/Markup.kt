package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.json.JSONArray
import org.json.JSONObject

/**
 * Marks drawn on a capture.
 *
 * Held in **page points, top-left origin** — the same space as annotations and as
 * the capture's crop, never in the pixels of the preview. The sheet lets the
 * export scale change after a mark is drawn, and a mark stored in pixels would
 * land a quarter of the way into the picture the moment it did.
 *
 * The split follows roadmap decision 4.7: the wet stroke and the preview drawing
 * are Kotlin's, the committed shape and the compositing are the engine's. What
 * leaves the app is drawn by the engine from the document and these shapes, so
 * nothing else can find its way into it.
 */
sealed interface MarkupShape {
    /** The stroke as drawn, unrecognised. */
    data class Freehand(val points: List<Offset>) : MarkupShape

    data class Line(val from: Offset, val to: Offset) : MarkupShape

    /** A line with a head at [to]. */
    data class Arrow(val from: Offset, val to: Offset) : MarkupShape

    data class Rectangle(val rect: Rect) : MarkupShape

    data class Ellipse(val rect: Rect) : MarkupShape

    /** A translucent wash, for picking something out rather than ringing it. */
    data class Highlight(val rect: Rect) : MarkupShape
}

/** One committed mark, with how it is drawn. */
data class Markup(
    val shape: MarkupShape,
    val color: Long,
    /** Stroke width in page points, so it keeps its weight at any export scale. */
    val widthPoints: Float = MARKUP_STROKE_POINTS,
)

/** Default nib, in page points. About a pen line on paper. */
const val MARKUP_STROKE_POINTS = 2.4f

/** What the markup toolbar can draw. */
enum class MarkupTool { Pen, Line, Arrow, Rectangle, Ellipse, Highlight }

/**
 * Whether this tool's shape is dragged corner to corner rather than traced.
 *
 * Everything but the pen is: the finger gives two points and the shape is built
 * from them, so the preview can be drawn from the drag alone with no stroke to
 * recognise afterwards.
 */
val MarkupTool.isDragged: Boolean get() = this != MarkupTool.Pen

/** Build the shape a drag defines, for every tool but the pen. */
fun MarkupTool.shapeFor(start: Offset, end: Offset): MarkupShape = when (this) {
    MarkupTool.Line -> MarkupShape.Line(start, end)
    MarkupTool.Arrow -> MarkupShape.Arrow(start, end)
    MarkupTool.Rectangle -> MarkupShape.Rectangle(rectFromCorners(start.x, start.y, end.x, end.y))
    MarkupTool.Ellipse -> MarkupShape.Ellipse(rectFromCorners(start.x, start.y, end.x, end.y))
    MarkupTool.Highlight -> MarkupShape.Highlight(rectFromCorners(start.x, start.y, end.x, end.y))
    // The pen traces rather than drags; its shape comes from the whole stroke.
    MarkupTool.Pen -> MarkupShape.Freehand(listOf(start, end))
}

// ------------------------------------------------------------------- the wire --

/**
 * The engine's form.
 *
 * Both sides pin this shape in a test with a literal string rather than sharing a
 * builder, because a shared builder lets the two agree on the wrong thing — which
 * is exactly how the annotation wire came to send `quarter_turns` to a decoder
 * expecting `quarterTurns`, with green tests either side.
 */
fun Markup.toWireJson(): JSONObject = JSONObject().apply {
    put("shape", shape.toWireJson())
    put("color", color.colorToWireJson())
    put("widthPt", widthPoints.toDouble())
}

fun List<Markup>.toWireJson(): String =
    JSONArray(map { it.toWireJson() }).toString()

private fun MarkupShape.toWireJson(): JSONObject = when (this) {
    is MarkupShape.Freehand -> JSONObject().apply {
        put("kind", "freehand")
        put("points", JSONArray(points.map { it.toWireJson() }))
    }
    is MarkupShape.Line -> JSONObject().apply {
        put("kind", "line")
        put("from", from.toWireJson())
        put("to", to.toWireJson())
    }
    is MarkupShape.Arrow -> JSONObject().apply {
        put("kind", "arrow")
        put("from", from.toWireJson())
        put("to", to.toWireJson())
    }
    is MarkupShape.Rectangle -> JSONObject().apply {
        put("kind", "rect")
        put("rect", rect.toMarkupWireJson())
    }
    is MarkupShape.Ellipse -> JSONObject().apply {
        put("kind", "ellipse")
        put("rect", rect.toMarkupWireJson())
    }
    is MarkupShape.Highlight -> JSONObject().apply {
        put("kind", "highlight")
        put("rect", rect.toMarkupWireJson())
    }
}

private fun Offset.toWireJson(): JSONObject = JSONObject().apply {
    put("x", x.toDouble())
    put("y", y.toDouble())
}

private fun Rect.toMarkupWireJson(): JSONObject = JSONObject().apply {
    put("left", left.toDouble())
    put("top", top.toDouble())
    put("right", right.toDouble())
    put("bottom", bottom.toDouble())
}

/** The points of a stroke, for the recogniser. */
fun List<Offset>.strokeToWireJson(): String =
    JSONArray(map { it.toWireJson() }).toString()

/**
 * Read back what the recogniser made of a stroke.
 *
 * An unrecognised kind comes back as freehand rather than throwing: a shape the
 * app does not know is still a mark someone drew, and losing it would be worse
 * than drawing it plainly.
 */
fun shapeFromWireJson(json: String, fallback: List<Offset>): MarkupShape {
    val root = runCatching { JSONObject(json) }.getOrNull()
        ?: return MarkupShape.Freehand(fallback)

    return when (root.optString("kind")) {
        "line" -> MarkupShape.Line(root.offset("from"), root.offset("to"))
        "arrow" -> MarkupShape.Arrow(root.offset("from"), root.offset("to"))
        "rect" -> MarkupShape.Rectangle(root.rect("rect"))
        "ellipse" -> MarkupShape.Ellipse(root.rect("rect"))
        "highlight" -> MarkupShape.Highlight(root.rect("rect"))
        "freehand" -> {
            val points = root.optJSONArray("points")
            if (points == null) {
                MarkupShape.Freehand(fallback)
            } else {
                MarkupShape.Freehand(
                    (0 until points.length()).map { i ->
                        val at = points.getJSONObject(i)
                        Offset(at.optDouble("x").toFloat(), at.optDouble("y").toFloat())
                    },
                )
            }
        }
        else -> MarkupShape.Freehand(fallback)
    }
}

private fun JSONObject.offset(name: String): Offset {
    val at = optJSONObject(name) ?: return Offset.Zero
    return Offset(at.optDouble("x").toFloat(), at.optDouble("y").toFloat())
}

private fun JSONObject.rect(name: String): Rect {
    val at = optJSONObject(name) ?: return Rect.Zero
    return Rect(
        left = at.optDouble("left").toFloat(),
        top = at.optDouble("top").toFloat(),
        right = at.optDouble("right").toFloat(),
        bottom = at.optDouble("bottom").toFloat(),
    )
}
