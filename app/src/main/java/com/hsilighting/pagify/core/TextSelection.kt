package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect

/**
 * Turns a drag across a page into the runs of text it selects.
 *
 * ## Why selection is a range over reading order, not a region of the page
 *
 * The obvious implementation — take every run whose vertical band the drag
 * swept — is wrong on any document with more than one column, and this one is a
 * two-column catalogue. On page 145 the left column's first line sits at
 * y 69.33 and the right column's first line at y 69.24: the same band. A
 * band filter cannot tell them apart, so highlighting one line of a paragraph
 * also highlighted the unrelated paragraph beside it. Measured on that page,
 * a single drag produced 266 to 431 rects out of the 468 runs on the page.
 *
 * Text has an order, and PDFium reports runs in it — [Page::textSegments] hands
 * them over in the document's own character order, which for this catalogue runs
 * down the left column, then down the right, then through the lower block. A
 * selection is therefore the *interval* between the run under one finger-down
 * point and the run under the release point, and the columns fall out for free:
 * the left column's runs occupy one contiguous stretch of that order, so a drag
 * inside it cannot reach the right column without passing through the end of the
 * left one.
 *
 * Only the two ends are trimmed horizontally, to where the drag actually began
 * and finished. Runs in between are taken whole, which is what selecting text
 * top-to-bottom means.
 *
 * ## Known limitation
 *
 * Inside a table, character order is cell order, which need not match what the
 * eye reads as adjacent. Dragging between two visually close cells can therefore
 * select the cells that lie between them in the document. Every viewer that
 * selects by character order behaves this way; the alternative is to reconstruct
 * a table model, which is roadmap phase D.
 *
 * ## Why this runs here rather than in the engine
 *
 * The model belongs in Rust as a rule, and this is a candidate to move down when
 * other platforms need it. It must not move *behind the render lock*: PDFium is
 * bound with `thread_safe`, so a call into the session serialises against page
 * rendering, which on this catalogue takes 300-780 ms. A drag emits an event per
 * frame, and each one would queue behind a render. The segments are already
 * cached page-side, so selecting over them costs nothing and never blocks.
 */
object TextSelection {

    /**
     * The rects to paint for a drag from [anchor] to [focus], both in page
     * points from the top-left.
     *
     * Direction does not matter: dragging up the page selects the same text as
     * dragging down it.
     */
    fun rectsBetween(
        segments: List<TextSegment>,
        anchor: Offset,
        focus: Offset,
    ): List<Rect> {
        val anchorIndex = indexNear(segments, anchor) ?: return emptyList()
        val focusIndex = indexNear(segments, focus) ?: return emptyList()

        // The interval is walked in reading order, so the point that trims the
        // left edge is whichever of the two came first in the text — not
        // whichever finger went down first.
        val forwards = anchorIndex <= focusIndex
        val first = if (forwards) anchorIndex else focusIndex
        val last = if (forwards) focusIndex else anchorIndex
        val startX = if (forwards) anchor.x else focus.x
        val endX = if (forwards) focus.x else anchor.x

        val rects = ArrayList<Rect>(last - first + 1)
        for (i in first..last) {
            val segment = segments[i]
            var left = segment.left
            var right = segment.right
            if (i == first) left = maxOf(left, startX)
            if (i == last) right = minOf(right, endX)

            // An empty intersection means the drag never covered this run
            // horizontally, and the run is simply not selected.
            //
            // The previous code normalised the two edges with min/max instead.
            // That silently turned "no overlap" — left past right — into a
            // positive-width rect spanning the gap between them, which is how a
            // drag inside one column painted a band reaching into the next.
            if (right - left <= MIN_WIDTH_POINTS) continue

            rects += Rect(left, segment.top, right, segment.bottom)
        }
        return rects
    }

    /**
     * The run nearest [point], preferring one it lands inside.
     *
     * Vertical distance is weighted, because a touch that misses the text is far
     * more likely to have missed it along the line it was aiming at than to have
     * meant the line above or below.
     */
    private fun indexNear(segments: List<TextSegment>, point: Offset): Int? {
        var best = -1
        var bestScore = Float.MAX_VALUE
        for (i in segments.indices) {
            val s = segments[i]
            val dy = when {
                point.y < s.top -> s.top - point.y
                point.y > s.bottom -> point.y - s.bottom
                else -> 0f
            }
            val dx = when {
                point.x < s.left -> s.left - point.x
                point.x > s.right -> point.x - s.right
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

    /** Below this a rect is a sliver no one asked for, usually a clipped edge. */
    private const val MIN_WIDTH_POINTS = 0.5f

    /** How much more a point missing vertically counts than missing sideways. */
    private const val VERTICAL_WEIGHT = 4f
}
