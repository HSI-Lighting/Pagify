package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect

/**
 * Where a page sits in the reader, and how big it is in its own points.
 *
 * [bounds] is in reader pixels — the same space a drag is reported in — so the
 * two can be intersected directly.
 */
data class PlacedPage(
    val pageIndex: Int,
    val bounds: Rect,
    val size: PageSize,
)

/**
 * Turn a dragged rectangle into one tile per page it touches.
 *
 * This is the arithmetic behind "capture what I can see, not what fits on one
 * page". The reader stacks pages in a column with gaps, so a box dragged around
 * something interesting routinely crosses a join: the bottom of one page, the gap,
 * and the top of the next. Capturing only the page the drag began on is what made
 * the feature feel broken.
 *
 * Kept here, pure, rather than inside a gesture handler, because it is the part
 * that can be wrong in ways nobody notices — a tile off by the height of a gap
 * looks perfectly plausible until you compare it with the screen.
 *
 * @param drag the rectangle, in reader pixels.
 * @param pages every page currently laid out, whether or not it is touched.
 */
fun captureTilesFor(drag: Rect, pages: List<PlacedPage>): List<CaptureTile> =
    pages.mapNotNull { page ->
        val visible = page.bounds.intersectOrNull(drag) ?: return@mapNotNull null
        if (visible.width < 1f || visible.height < 1f) return@mapNotNull null

        // Pixels per point, for this page. Per page rather than shared: the reader
        // draws every page to the same width, so an A3 sheet and an A5 one are at
        // very different scales on the same screen.
        val horizontal = page.bounds.width / page.size.widthPoints
        val vertical = page.bounds.height / page.size.heightPoints
        // `isFinite` as well as positive: a page whose size has not been measured
        // yet is zero by zero, and dividing by that gives infinity rather than
        // anything a `<= 0` check would catch. The tile it produced looked valid
        // and cropped nothing.
        if (!horizontal.isFinite() || !vertical.isFinite()) return@mapNotNull null
        if (horizontal <= 0f || vertical <= 0f) return@mapNotNull null

        CaptureTile(
            pageIndex = page.pageIndex,
            crop = Rect(
                left = (visible.left - page.bounds.left) / horizontal,
                top = (visible.top - page.bounds.top) / vertical,
                right = (visible.right - page.bounds.left) / horizontal,
                bottom = (visible.bottom - page.bounds.top) / vertical,
            ),
            // Relative to the drag, because the picture's origin is the drag's
            // top-left and not the reader's.
            dest = visible.translate(-drag.left, -drag.top),
        )
    }

/** The overlap, or null when there is none. */
private fun Rect.intersectOrNull(other: Rect): Rect? {
    val left = maxOf(left, other.left)
    val top = maxOf(top, other.top)
    val right = minOf(right, other.right)
    val bottom = minOf(bottom, other.bottom)
    return if (right > left && bottom > top) Rect(left, top, right, bottom) else null
}

/**
 * Where the magnified page sits on screen, in the zoomed view's own pixels.
 *
 * Trivial arithmetic, pulled out of the composable so it can be tested at all:
 * inside a `Modifier` chain it is reachable only on a device, and it is the one
 * place the zoomed capture can be wrong. A capture whose crop is derived from the
 * wrong rectangle still produces a plausible picture — of the wrong part of the
 * page.
 *
 * The zoomed view draws the page translated by [offset] and scaled about its own
 * top-left, so its rectangle is the offset and the scaled size. [offset] is
 * negative once the page is larger than the viewport, which is the normal case
 * when zoomed in.
 */
fun zoomedPageBounds(
    offset: Offset,
    baseWidthPx: Float,
    baseHeightPx: Float,
    scale: Float,
): Rect = Rect(
    left = offset.x,
    top = offset.y,
    right = offset.x + baseWidthPx * scale,
    bottom = offset.y + baseHeightPx * scale,
)

/**
 * The rectangle a drawn ring will be captured as.
 *
 * An image is a rectangle, so a lasso still produces one: its bounding box. What
 * the ring changes is what survives *inside* that box — see [captureMaskFor].
 *
 * Returns null for anything that does not enclose a usable area, which is a
 * stricter question than "is the box big enough". Measured on a phone: a drag
 * straight across the page in lasso mode passes every size check — a 500-pixel
 * box — and encloses nothing at all, so the capture came back blank and the
 * editor opened on an empty grey rectangle. The area is what the ring means.
 */
fun lassoBounds(outline: List<Offset>): Rect? {
    if (outline.size < 3) return null
    var left = Float.MAX_VALUE
    var top = Float.MAX_VALUE
    var right = -Float.MAX_VALUE
    var bottom = -Float.MAX_VALUE
    for (point in outline) {
        if (!point.x.isFinite() || !point.y.isFinite()) return null
        left = minOf(left, point.x)
        top = minOf(top, point.y)
        right = maxOf(right, point.x)
        bottom = maxOf(bottom, point.y)
    }
    val box = Rect(left, top, right, bottom)
    if (!box.isWorthCapturing()) return null

    val enclosed = enclosedArea(outline)
    // Two floors, because they catch different mistakes. The absolute one rejects
    // a ring around nothing; the proportional one rejects a long thin smear, which
    // has plenty of absolute area and still shows almost nothing of the page.
    if (enclosed < MINIMUM_CAPTURE_POINTS * MINIMUM_CAPTURE_POINTS) return null
    if (enclosed < box.width * box.height * MINIMUM_RING_FULLNESS) return null
    return box
}

/**
 * How much a drawn ring actually encloses, by the shoelace formula.
 *
 * Unsigned, because a ring drawn anticlockwise encloses exactly as much as the
 * same ring drawn clockwise. Closed for us, as everywhere else the ring is
 * treated: the last point joins the first.
 */
private fun enclosedArea(outline: List<Offset>): Float {
    var twiceArea = 0f
    for (index in outline.indices) {
        val here = outline[index]
        val next = outline[(index + 1) % outline.size]
        twiceArea += here.x * next.y - next.x * here.y
    }
    return kotlin.math.abs(twiceArea) / 2f
}

/**
 * The least of its own bounding box a ring may enclose.
 *
 * Low on purpose: a ring around an L-shaped detail, or a crescent along the edge
 * of a drawing, is a legitimate shape that fills very little of its box. This is
 * only here to reject the shapes that enclose *nothing* — a straight drag, a
 * scribble back and forth — which no amount of size checking catches.
 */
private const val MINIMUM_RING_FULLNESS = 0.02f

/**
 * A drawn ring in the picture's own coordinates.
 *
 * The same move `captureTilesFor` makes for a tile's destination: the picture's
 * origin is the drag's top-left, not the reader's, so every point shifts by the
 * bounding box's corner. Without it the ring would be masked against the top-left
 * of the screen and the capture would come back almost entirely blank — the kind
 * of mistake that produces a plausible picture of the wrong thing.
 */
fun captureMaskFor(drag: Rect, outline: List<Offset>): List<Offset> =
    if (outline.size < 3) emptyList() else outline.map { it - drag.topLeft }
