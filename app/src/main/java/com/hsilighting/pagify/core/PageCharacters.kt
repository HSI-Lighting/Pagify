package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.json.JSONObject

/**
 * A page's text with a box for every character of it.
 *
 * The foundation of selecting text, and the reason [TextSegment] is not: a run
 * is a whole line, so a selection built from runs can only begin and end at a
 * line. Dragging across half a sentence would copy both lines it touched, whole.
 *
 * [boxes] holds four floats per character of [text] — left, top, right, bottom,
 * in page points from the top-left — aligned to it by construction, since the
 * engine builds both from one walk of the page. The alignment is in UTF-16 code
 * units, which is how Kotlin indexes a string, so `text[i]` and the box at `i`
 * are the same character even outside the basic plane.
 */
class PageCharacters(val text: String, val boxes: FloatArray) {

    val count: Int get() = text.length

    val isEmpty: Boolean get() = count == 0

    fun boxAt(index: Int): Rect {
        val at = index * 4
        return Rect(boxes[at], boxes[at + 1], boxes[at + 2], boxes[at + 3])
    }

    /**
     * The character nearest [point], or null on a page with no text.
     *
     * Vertical distance is weighted, for the same reason it is when hit-testing a
     * run: a touch that misses the text almost always missed along the line it
     * was aiming at, rather than meaning the line above or below.
     */
    fun indexNear(point: Offset): Int? {
        if (isEmpty) return null

        var best = -1
        var bestScore = Float.MAX_VALUE
        for (i in 0 until count) {
            val box = boxAt(i)
            val dy = when {
                point.y < box.top -> box.top - point.y
                point.y > box.bottom -> point.y - box.bottom
                else -> 0f
            }
            val dx = when {
                point.x < box.left -> box.left - point.x
                point.x > box.right -> point.x - box.right
                else -> 0f
            }
            val score = dy * VERTICAL_WEIGHT + dx
            if (score < bestScore) {
                bestScore = score
                best = i
            }
        }
        return best.takeIf { it >= 0 }
    }

    /**
     * The word around [index], as a half-open range.
     *
     * What a long press should select. Landing on a single character is almost
     * never what someone meant — they pointed at a word — and starting from the
     * whole word means the handles are usually only nudged rather than dragged
     * across the line.
     */
    fun wordAround(index: Int): IntRange {
        if (isEmpty) return IntRange.EMPTY
        val at = index.coerceIn(0, count - 1)
        if (text[at].isWhitespace()) return at..at

        var start = at
        while (start > 0 && !text[start - 1].isWhitespace()) start--
        var end = at
        while (end < count - 1 && !text[end + 1].isWhitespace()) end++
        return start..end
    }

    /** The text of a selection, exactly as the page holds it. */
    fun textOf(range: IntRange): String {
        if (isEmpty || range.isEmpty()) return ""
        val from = range.first.coerceIn(0, count - 1)
        val to = range.last.coerceIn(from, count - 1)
        return text.substring(from, to + 1)
    }

    /**
     * The rectangles to paint over a selection: one per line, not one per
     * character.
     *
     * Consecutive characters that share a line are merged, which is what makes a
     * selection look like a band over the text rather than a row of separate
     * boxes — and it is also far less to draw on a page where a selection can run
     * to thousands of characters.
     *
     * A gap in the middle of a line is kept as a gap. Two columns of a table can
     * sit on one line with empty space between them, and painting across that
     * space would claim text that is not selected.
     */
    fun rectsOf(range: IntRange): List<Rect> {
        if (isEmpty || range.isEmpty()) return emptyList()
        val from = range.first.coerceIn(0, count - 1)
        val to = range.last.coerceIn(from, count - 1)

        val rects = mutableListOf<Rect>()
        var current: Rect? = null

        for (i in from..to) {
            if (text[i] == '\n' || text[i] == '\r') continue
            val box = boxAt(i)
            if (box.width <= 0f && box.height <= 0f) continue

            val previous = current
            current = if (previous != null && box.joinsOnTheSameLine(previous)) {
                Rect(
                    left = minOf(previous.left, box.left),
                    top = minOf(previous.top, box.top),
                    right = maxOf(previous.right, box.right),
                    bottom = maxOf(previous.bottom, box.bottom),
                )
            } else {
                previous?.let { rects += it }
                box
            }
        }
        current?.let { rects += it }
        return rects
    }

    /**
     * Whether a character continues the band being built.
     *
     * Two tests, and both are needed. The vertical one catches a new line; the
     * horizontal one catches a jump across a page — the next column, or the far
     * side of a table — which sits on the same line and must not be painted
     * through.
     */
    private fun Rect.joinsOnTheSameLine(previous: Rect): Boolean {
        val sameLine = kotlin.math.abs(top - previous.top) <= previous.height * LINE_TOLERANCE
        val adjacent = left <= previous.right + previous.height * GAP_TOLERANCE &&
            right >= previous.left - previous.height * GAP_TOLERANCE
        return sameLine && adjacent
    }

    companion object {
        fun fromJson(json: String): PageCharacters {
            val root = JSONObject(json)
            val array = root.optJSONArray("boxes")
            val boxes = FloatArray(array?.length() ?: 0)
            for (i in boxes.indices) boxes[i] = array!!.getDouble(i).toFloat()
            return PageCharacters(root.optString("text"), boxes)
        }

        val EMPTY = PageCharacters("", FloatArray(0))

        /** How much more a touch missing vertically counts than missing sideways. */
        private const val VERTICAL_WEIGHT = 4f

        /** How far two characters' tops may differ and still be one line. */
        private const val LINE_TOLERANCE = 0.5f

        /**
         * How wide a gap may be, as a multiple of the line height, before it is
         * treated as a jump rather than a space.
         *
         * A space is well under one line height; the gutter between two columns
         * is several.
         */
        private const val GAP_TOLERANCE = 1.5f
    }
}
