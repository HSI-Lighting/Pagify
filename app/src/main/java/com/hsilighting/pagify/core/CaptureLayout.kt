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
