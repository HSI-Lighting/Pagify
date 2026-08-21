package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.json.JSONArray
import org.json.JSONObject

/**
 * Marks drawn on a capture.
 *
 * Held in **capture-local units** — the picture's own space, origin at its
 * top-left — rather than in any page's points. A capture can span two pages, and
 * a mark drawn across the join belongs to neither of them. Nor in the preview's
 * pixels: the editor lets the export scale change after a mark is drawn, and a
 * mark stored in pixels would land somewhere else entirely the moment it did.
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
    /** Solid, dashed or dash-dot. Only the line tool offers anything but solid. */
    val style: MarkupStyle = MarkupStyle.SOLID,
)

/**
 * How a stroke is broken up along its length.
 *
 * The four broken types are the drawing-office conventions, because that is what
 * they are for: on a plan a dashed line means something — a hidden edge, a level
 * above, a centre — and a mark that looks *nearly* like the convention reads as a
 * mistake to anyone who works from drawings.
 *
 * Applies to everything that draws a line: the pen, the line, the arrow, the box
 * and the circle. Not the highlighter, which is a filled wash.
 */
enum class MarkupStyle(val label: String, val wireName: String) {
    SOLID("Solid", "solid"),
    DASH_1("Dash-1", "dash1"),
    DASH_2("Dash-2", "dash2"),
    CENTERLINE_1("Centerline-1", "centerline1"),
    CENTERLINE_2("Centerline-2", "centerline2"),
}

/** Whether this is a broken line rather than a plain one. */
val MarkupStyle.isBroken: Boolean get() = this != MarkupStyle.SOLID

/**
 * Whether this tool draws in a line type at all.
 *
 * Everything but the highlighter: a wash has no length to break up.
 */
val MarkupTool.hasLineStyle: Boolean get() = !isIntensity
/** Default nib, in capture units. About a pen line on paper. */
const val MARKUP_STROKE_POINTS = 2.4f

/** What the markup toolbar can draw. */
enum class MarkupTool { Pen, Line, Arrow, Rectangle, Ellipse, Cloud, Highlight }

/**
 * The two ways of ringing a region, in the order they are always offered.
 *
 * As in the reader, and for the same reasons: they answer one question, so they
 * share one slot, and the order is fixed so the press that opens them and the tap
 * that follows always land on the same thing. It costs the circle its fine-size
 * slider — the three presets are still there — which is the price of the two
 * toolbars reaching the cloud by the same press.
 */
val RING_MARKUP_TOOLS = listOf(MarkupTool.Ellipse, MarkupTool.Cloud)

/** The tools sharing this one's place in the row, itself included. */
val MarkupTool.slotMates: List<MarkupTool>
    get() = if (this in RING_MARKUP_TOOLS) RING_MARKUP_TOOLS else listOf(this)

/** The other tool sharing this one's place in the row, if it shares one. */
val MarkupTool.alternate: MarkupTool? get() = slotMates.firstOrNull { it != this }

/**
 * How heavy a tool draws.
 *
 * One number per tool, meaning two different things: for anything that draws a
 * line it is the **nib width**, and for the highlighter it is the **intensity**
 * of the wash, 0..1. They are the same control answering the same question —
 * "how strong is this tool" — so they are one setting rather than two, and only
 * the range and the label differ.
 *
 * Per tool rather than shared: someone who has set a fine pen does not expect
 * picking up the highlighter and putting it down again to have changed it.
 */
val MarkupTool.isIntensity: Boolean get() = this == MarkupTool.Highlight

/** Where a tool's slider starts and stops. */
val MarkupTool.sizeRange: ClosedFloatingPointRange<Float>
    get() = if (isIntensity) 0.08f..0.85f else 0.6f..16f

/** What a tool draws at until it is changed. */
val MarkupTool.defaultSize: Float get() = if (isIntensity) 0.35f else MARKUP_STROKE_POINTS

/**
 * The sizes offered as a tap rather than a drag.
 *
 * Three, spread across the range, and always on screen: thin, medium and thick
 * is the whole of what most marks need, and a size that takes a press to reach
 * is a size nobody changes. The slider — behind a long press on the tool — is
 * for when none of the three is quite right.
 */
val MarkupTool.sizePresets: List<Float>
    get() = if (isIntensity) {
        listOf(0.2f, 0.4f, 0.65f)
    } else {
        listOf(1.2f, 3f, 8f)
    }

/** Every tool at its default, for a fresh capture. */
fun defaultMarkupSizes(): Map<MarkupTool, Float> =
    MarkupTool.entries.associateWith { it.defaultSize }

/**
 * Build a mark from the tool that drew it and how heavy that tool is set to.
 *
 * The highlighter's intensity rides in the colour's **alpha**, which is where the
 * engine reads it: a wash and a stroke are the same `Markup`, and giving the
 * highlighter a field of its own would mean every other tool carrying one it
 * ignores.
 */
fun markupFor(
    shape: MarkupShape,
    tool: MarkupTool,
    color: Long,
    size: Float,
    style: MarkupStyle = MarkupStyle.SOLID,
): Markup =
    if (tool.isIntensity) {
        Markup(
            shape = shape,
            color = (color and 0x00FFFFFFL) or ((size.coerceIn(0f, 1f) * 255f).toLong() shl 24),
            widthPoints = 0f,
        )
    } else {
        Markup(
            shape = shape,
            color = color,
            widthPoints = size,
            // Carried only by the tool that offers it, so a style chosen for
            // lines does not quietly follow you to the box tool.
            style = if (tool.hasLineStyle) style else MarkupStyle.SOLID,
        )
    }

/**
 * Whether this tool follows the finger rather than taking two corners.
 *
 * The pen keeps the path as drawn; the cloud replaces it with scallops. Either
 * way the whole stroke matters, so neither can be previewed from the drag alone.
 */
val MarkupTool.tracesPath: Boolean get() = this == MarkupTool.Pen || this == MarkupTool.Cloud

/**
 * Whether this tool's shape is dragged corner to corner rather than traced.
 *
 * The finger gives two points and the shape is built from them, so the preview
 * can be drawn from the drag alone with no stroke to recognise afterwards.
 */
val MarkupTool.isDragged: Boolean get() = !tracesPath

/**
 * Whether holding still at the end of a stroke asks for the shape recogniser.
 *
 * The pen only. A cloud is already the shape it is going to be — asking the
 * engine what a hand-traced ring "really" was would turn it into an ellipse, and
 * an ellipse is the one thing a cloud is deliberately not.
 */
val MarkupTool.recognises: Boolean get() = this == MarkupTool.Pen

/** Build the shape a drag defines, for every tool but the traced ones. */
fun MarkupTool.shapeFor(start: Offset, end: Offset): MarkupShape = when (this) {
    MarkupTool.Line -> MarkupShape.Line(start, end)
    MarkupTool.Arrow -> MarkupShape.Arrow(start, end)
    MarkupTool.Rectangle -> MarkupShape.Rectangle(rectFromCorners(start.x, start.y, end.x, end.y))
    MarkupTool.Ellipse -> MarkupShape.Ellipse(rectFromCorners(start.x, start.y, end.x, end.y))
    MarkupTool.Highlight -> MarkupShape.Highlight(rectFromCorners(start.x, start.y, end.x, end.y))
    // These trace rather than drag; their shape comes from the whole stroke.
    MarkupTool.Pen, MarkupTool.Cloud -> MarkupShape.Freehand(listOf(start, end))
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
    put("style", style.wireName)
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

internal fun Rect.toMarkupWireJson(): JSONObject = JSONObject().apply {
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

// ------------------------------------------------------------- capture tiles --

/**
 * The tiles a capture is assembled from, in the engine's form.
 *
 * `pageIndex` rather than `page_index`: the engine renames struct fields to camel
 * case on this wire. Pinned in a test on both sides, because a field name that
 * does not match fails at run time on a device and nowhere else.
 */
fun List<CaptureTile>.tilesToWireJson(): String = JSONArray(
    map { tile ->
        JSONObject().apply {
            put("pageIndex", tile.pageIndex)
            put("crop", tile.crop.toMarkupWireJson())
            put("dest", tile.dest.toMarkupWireJson())
        }
    },
).toString()
