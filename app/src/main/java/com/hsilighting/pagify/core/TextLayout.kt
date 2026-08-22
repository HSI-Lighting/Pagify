package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.sin

/**
 * Where one glyph sits, and which way it leans.
 *
 * [origin] is the glyph's own origin — the left end of its baseline — not its
 * centre, because that is what both Android and PDF place a glyph by. [radians]
 * is measured the way the app's coordinates run, clockwise with y downward.
 */
data class GlyphPlacement(
    val character: Char,
    val origin: Offset,
    val radians: Float,
)

/**
 * Text laid along a baseline, glyph by glyph.
 *
 * One routine, used by everything: the preview on screen, the glyphs written into
 * the PDF, and the letters drawn onto a screenshot. That is the whole reason it
 * exists — three separate layouts would agree until they did not, and the way that
 * shows up is text that lands somewhere other than where it was placed.
 *
 * Advances come from the font's own metrics ([advanceOf]), never from what the
 * phone measures. The phone does not have Helvetica; it has something that looks
 * like it, and laying out from *that* would put the preview and the file half a
 * word apart by the end of a line.
 *
 * A straight baseline is just a path of two points, so there is no separate case
 * for it — the same walk handles a line and a curve.
 *
 * Glyphs that run off the end of the path are dropped. Bunching them at the last
 * point would draw a smear of overlapping letters, which reads as a bug; running
 * on past the end would put text where no line was drawn.
 */
fun layOutText(
    text: String,
    font: PdfFont,
    sizePoints: Float,
    path: List<Offset>,
): List<GlyphPlacement> {
    if (text.isEmpty() || path.size < 2) return emptyList()

    val placements = mutableListOf<GlyphPlacement>()
    var travelled = 0f

    text.forEach { character ->
        val at = pointAlong(path, travelled) ?: return placements
        placements += GlyphPlacement(character, at.first, at.second)
        travelled += font.advanceOf(character, sizePoints)
    }

    return placements
}

/** How far along the baseline the text reaches, and how far there is to go. */
fun baselineLength(path: List<Offset>): Float =
    path.zipWithNext().sumOf { (a, b) -> (b - a).getDistance().toDouble() }.toFloat()

/**
 * The point [distance] along the path, and the direction the path runs there.
 *
 * Null once the path runs out, which is what stops text overflowing the line it
 * was drawn on.
 */
private fun pointAlong(path: List<Offset>, distance: Float): Pair<Offset, Float>? {
    if (distance < 0f) return null

    var travelled = 0f
    for ((from, to) in path.zipWithNext()) {
        val span = to - from
        val length = span.getDistance()
        if (length <= 0f) continue

        if (travelled + length >= distance) {
            val along = (distance - travelled) / length
            return (from + span * along) to atan2(span.y, span.x)
        }
        travelled += length
    }
    return null
}

/**
 * A baseline for text placed by a single tap, running to the right of it.
 *
 * Long enough for the text and no longer. The baseline is what the layout walks,
 * so it has to exist even when nobody drew one — and making it exactly as long as
 * the text keeps "the text ran off the end" from ever meaning something different
 * for straight text than for curved.
 */
fun straightBaseline(anchor: Offset, text: String, font: PdfFont, sizePoints: Float): List<Offset> =
    listOf(anchor, anchor + Offset(font.widthOf(text, sizePoints), 0f))

/**
 * The box a framed mark's frame is drawn around.
 *
 * Measured from the words: their advance width, the height a line of that size
 * occupies, and a margin proportional to the size. That proportion is the whole
 * point of the tool — set the point size and the cloud follows, so there is no
 * second thing to keep in step.
 */
fun Annotation.Text.textFrameBounds(): Rect =
    textFrameBounds(path.firstOrNull() ?: Offset.Zero, font.widthOf(text, sizePoints), sizePoints)

/**
 * The same, for anything holding the same three numbers.
 *
 * Taken apart from the mark so that a screenshot's text and a page's text are
 * framed by one piece of arithmetic. Two copies of this would be two frames that
 * agreed until somebody changed the margin.
 */
fun textFrameBounds(anchor: Offset, runWidth: Float, sizePoints: Float): Rect {
    val margin = sizePoints * CLOUD_TEXT_MARGIN
    return Rect(
        left = anchor.x - margin,
        top = anchor.y - sizePoints * CAP_HEIGHT - margin,
        right = anchor.x + runWidth + margin,
        bottom = anchor.y + sizePoints * DESCENDER + margin,
    )
}

/**
 * The ring drawn around a framed mark, as one closed polyline.
 *
 * The cloud comes through the same [cloudOutline] the cloud tool uses, so a
 * cloud round words and a cloud drawn by hand are the same notation. Its bump
 * size is taken from the point size rather than from a nib width: this cloud has
 * no stroke of its own to scale by.
 *
 * The ellipse is grown by root two rather than drawn on the box itself. An
 * ellipse *inscribed* in a rectangle cuts every corner off it, which on a line of
 * text means clipping the first and last letters — so it is scaled until the box
 * fits inside it instead.
 */
fun Annotation.Text.textFrameOutline(): List<Offset> =
    textFrameOutline(textFrameBounds(), sizePoints, frame)

/** The ring around a box already measured. */
fun textFrameOutline(box: Rect, sizePoints: Float, frame: TextFrame): List<Offset> {
    val corners = listOf(
        Offset(box.left, box.top),
        Offset(box.right, box.top),
        Offset(box.right, box.bottom),
        Offset(box.left, box.bottom),
    )
    return when (frame) {
        TextFrame.None -> emptyList()
        TextFrame.Cloud -> cloudOutline(corners, widthPoints = sizePoints * CLOUD_TEXT_BUMP)
        TextFrame.Box -> corners + corners.first()
        TextFrame.Ellipse -> ellipseThrough(box)
    }
}

/** An ellipse the size of the box's circumscribing one, as a closed polyline. */
private fun ellipseThrough(box: Rect): List<Offset> {
    val centre = box.center
    val radiusX = box.width / 2f * ELLIPSE_REACH
    val radiusY = box.height / 2f * ELLIPSE_REACH
    return (0..ELLIPSE_SEGMENTS).map { step ->
        val angle = step.toFloat() / ELLIPSE_SEGMENTS * 2f * PI.toFloat()
        Offset(centre.x + cos(angle) * radiusX, centre.y + sin(angle) * radiusY)
    }
}

/** How many segments an ellipse is drawn in. Enough that no flat side shows. */
private const val ELLIPSE_SEGMENTS = 64

/**
 * How far past half the box the ellipse reaches.
 *
 * Root two puts the corners exactly *on* the curve, and a curve drawn as 64
 * chords cuts inside itself between samples — so at exactly root two the corner
 * letters sit a hair outside the ring that is supposed to contain them. The
 * extra few percent is what makes the drawn ring enclose the drawn words.
 */
private const val ELLIPSE_REACH = 1.47f

/** How thick a stroke the cloud's bumps are sized as, per point of type. */
private const val CLOUD_TEXT_BUMP = 0.17f

/** The margin round the words, per point of type. */
private const val CLOUD_TEXT_MARGIN = 0.45f

/**
 * Cap height and descender as fractions of the point size.
 *
 * The standard-14 faces differ by a percent or two and the cloud does not care:
 * this decides where a box sits around a line of type, not where a glyph goes.
 */
private const val CAP_HEIGHT = 0.72f

private const val DESCENDER = 0.21f

/**
 * A baseline that bends, for text placed by a single tap.
 *
 * The reader no longer draws the curve: they say how much bend they want and the
 * app makes the arc. Drawing it by hand looked broken for a reason that could not
 * be fixed by drawing more carefully — a short caption covers only the first part
 * of a long stroke, and the first part of any hand-drawn arc is its straightest.
 *
 * [degrees] is how far the line turns from end to end: 0 is straight, positive
 * arches upward, negative sags. The arc is exactly as long as the words, so every
 * letter lands on it however far it bends, and it is symmetric about the
 * horizontal so that turning the bend up does not also swing the words sideways.
 */
fun curvedBaseline(
    anchor: Offset,
    text: String,
    font: PdfFont,
    sizePoints: Float,
    degrees: Float,
): List<Offset> {
    val width = font.widthOf(text, sizePoints)
    val turn = Math.toRadians(degrees.toDouble()).toFloat()
    // Straight enough that an arc would only add rounding error.
    if (width <= 0f || abs(turn) < STRAIGHT_ENOUGH) {
        return straightBaseline(anchor, text, font, sizePoints)
    }

    val radius = width / turn
    val start = -turn / 2f
    val steps = (abs(turn) / ARC_STEP).toInt().coerceIn(2, MAXIMUM_ARC_STEPS)

    return (0..steps).map { step ->
        val along = start + turn * step / steps
        Offset(
            anchor.x + radius * (sin(along) - sin(start)),
            anchor.y + radius * (cos(start) - cos(along)),
        )
    }
}

/** Below this much turn the arc and the straight line differ by less than a hair. */
private const val STRAIGHT_ENOUGH = 0.02f

/** About three degrees a segment, so no flat spot shows at any size. */
private const val ARC_STEP = 0.05f

private const val MAXIMUM_ARC_STEPS = 128

/**
 * The same words at a different size.
 *
 * The baseline is scaled about the point the text was placed at, by the same
 * factor as the type. It has to be: the layout walks that line and drops any
 * glyph that runs off the end, so type that grew on a line that did not would
 * lose its last letters — and a bent line keeps its bend, because scaling about
 * a point does not change any angle.
 *
 * The size is held between [MINIMUM_TEXT_POINTS] and [MAXIMUM_TEXT_POINTS], and
 * the line follows the size it actually reached rather than the one asked for.
 */
fun Annotation.Text.scaledBy(factor: Float): Annotation.Text {
    val anchor = path.firstOrNull() ?: return this
    val wanted = sizePoints * factor
    val reached = wanted.coerceIn(MINIMUM_TEXT_POINTS, MAXIMUM_TEXT_POINTS)
    if (reached == sizePoints) return this

    val achieved = reached / sizePoints
    return copy(
        sizePoints = reached,
        path = path.map { anchor + (it - anchor) * achieved },
    )
}

/** As above, for words on a picture. */
fun MarkupShape.Text.scaledBy(factor: Float): MarkupShape.Text {
    val anchor = path.firstOrNull() ?: return this
    val reached = (sizePoints * factor).coerceIn(MINIMUM_TEXT_POINTS, MAXIMUM_TEXT_POINTS)
    if (reached == sizePoints) return this

    val achieved = reached / sizePoints
    return copy(
        sizePoints = reached,
        path = path.map { anchor + (it - anchor) * achieved },
    )
}

/** Smaller than this and the words are a smudge; larger and they are a poster. */
const val MINIMUM_TEXT_POINTS = 6f

/**
 * The backstop, well past anything a page allows.
 *
 * The ceiling that actually bites is the page itself — see [PdfFont.sizeThatFits]
 * — because type is only as big as useful while the words still fit across the
 * sheet. This is here so nothing runs away when there is no page to ask.
 */
const val MAXIMUM_TEXT_POINTS = 400f

/**
 * The same caption, restyled.
 *
 * The baseline is rebuilt rather than adjusted, because everything about it
 * follows from the other four: where it starts, what it says, how big, and how
 * far it bends. Adjusting it in place is how a caption ends up in a face that no
 * longer fits the line it sits on, losing its last letters — the layout walks the
 * line and drops any glyph that runs off the end.
 */
fun Annotation.Text.rebuilt(
    text: String = this.text,
    font: PdfFont = this.font,
    sizePoints: Float = this.sizePoints,
    curveDegrees: Float = this.curveDegrees,
    color: Long = this.color,
    frame: TextFrame = this.frame,
): Annotation.Text {
    val anchor = path.firstOrNull() ?: Offset.Zero
    val size = sizePoints.coerceIn(MINIMUM_TEXT_POINTS, MAXIMUM_TEXT_POINTS)
    return copy(
        text = text,
        font = font,
        sizePoints = size,
        curveDegrees = curveDegrees,
        color = color,
        frame = frame,
        path = curvedBaseline(anchor, text, font, size, curveDegrees),
    )
}

/** The same, for a caption on a picture. */
fun MarkupShape.Text.rebuiltMarkup(
    text: String = this.text,
    font: PdfFont = this.font,
    sizePoints: Float = this.sizePoints,
    curveDegrees: Float = this.curveDegrees,
    frame: TextFrame = this.frame,
): MarkupShape.Text {
    val anchor = path.firstOrNull() ?: Offset.Zero
    val size = sizePoints.coerceIn(MINIMUM_TEXT_POINTS, MAXIMUM_TEXT_POINTS)
    return copy(
        text = text,
        font = font,
        sizePoints = size,
        curveDegrees = curveDegrees,
        frame = frame,
        path = curvedBaseline(anchor, text, font, size, curveDegrees),
    )
}
