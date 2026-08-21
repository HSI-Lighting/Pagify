package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect

/**
 * Keeping marks attached to the right page when the page tree changes underneath
 * them.
 *
 * Annotations are addressed by page *index*, and every structural edit renumbers
 * pages: deleting page 2 makes the old page 3 into the new page 2, and a reorder
 * moves every page at once. Without a remap, a user who deletes one page finds
 * their highlights have slid onto the wrong pages — silently, with nothing on
 * screen to say it happened.
 *
 * Rotation renumbers nothing but changes the page's coordinate space, so marks on
 * a rotated page need their geometry turned with it. Both live here because they
 * are the same obligation: an edit to the document has to carry its annotations
 * along, or it corrupts them.
 */

/** The same mark, attributed to a different page. */
fun Annotation.movedTo(pageIndex: Int): Annotation = when (this) {
    is Annotation.Highlight -> copy(pageIndex = pageIndex)
    is Annotation.Ink -> copy(pageIndex = pageIndex)
    is Annotation.Note -> copy(pageIndex = pageIndex)
    is Annotation.Shape -> copy(pageIndex = pageIndex)
    is Annotation.Signature -> copy(pageIndex = pageIndex)
}

/**
 * Turn a point clockwise within a page of [width] x [height] points.
 *
 * The page's own dimensions are the pivot, not the origin: a quarter turn maps a
 * `width x height` page onto a `height x width` one, so the coordinate that gets
 * mirrored is the one whose extent changed. Checking a corner is the quickest way
 * to see it is right — the top-left `(0, 0)` of a page turned 90 degrees ends up
 * at the top-*right*, which is `(height, 0)` in the turned page's own space.
 */
fun Offset.rotatedInPage(quarterTurns: Int, width: Float, height: Float): Offset =
    when (((quarterTurns % 4) + 4) % 4) {
        1 -> Offset(height - y, x)
        2 -> Offset(width - x, height - y)
        3 -> Offset(y, width - x)
        else -> this
    }

/**
 * Turn a rect, then put its corners back in order.
 *
 * Rotating corner-by-corner can leave `left > right`, which [Rect] happily stores
 * and every hit test then quietly fails against. Rebuilding from the extremes is
 * what keeps an inverted rect from becoming an invisible highlight.
 */
fun Rect.rotatedInPage(quarterTurns: Int, width: Float, height: Float): Rect {
    val a = Offset(left, top).rotatedInPage(quarterTurns, width, height)
    val b = Offset(right, bottom).rotatedInPage(quarterTurns, width, height)
    return Rect(
        left = minOf(a.x, b.x),
        top = minOf(a.y, b.y),
        right = maxOf(a.x, b.x),
        bottom = maxOf(a.y, b.y),
    )
}

/** The same mark, turned with its page. [width] and [height] are the page's size
 *  *before* the turn. */
fun Annotation.rotatedInPage(quarterTurns: Int, width: Float, height: Float): Annotation {
    if (((quarterTurns % 4) + 4) % 4 == 0) return this
    return when (this) {
        is Annotation.Highlight ->
            copy(rects = rects.map { it.rotatedInPage(quarterTurns, width, height) })

        is Annotation.Ink ->
            copy(points = points.map { it.rotatedInPage(quarterTurns, width, height) })

        is Annotation.Note ->
            copy(anchor = anchor.rotatedInPage(quarterTurns, width, height))

        is Annotation.Shape -> copy(
            strokes = strokes.map { stroke ->
                stroke.map { it.rotatedInPage(quarterTurns, width, height) }
            },
        )

        is Annotation.Signature -> {
            val turned = strokes.map { stroke ->
                stroke.map { it.rotatedInPage(quarterTurns, width, height) }
            }
            copy(strokes = turned, bounds = bounds.rotatedInPage(quarterTurns, width, height))
        }
    }
}

/**
 * How a page-tree edit renumbers pages.
 *
 * `null` means the page is gone and its marks go with it. Built from the same
 * parameters as the [PdfCommand] that caused the change, so the two cannot drift
 * apart in the ways a hand-written mapping per call site would.
 */
fun interface PageRemap {
    /** The new index of [oldIndex], or null if that page no longer exists. */
    operator fun invoke(oldIndex: Int): Int?

    companion object {
        fun afterDelete(deleted: Int) = PageRemap { old ->
            when {
                old == deleted -> null
                old > deleted -> old - 1
                else -> old
            }
        }

        fun afterInsert(at: Int) = PageRemap { old -> if (old >= at) old + 1 else old }

        /** [order] is a destination map: `order[i]` is where page `i` ends up. */
        fun afterReorder(order: List<Int>) = PageRemap { old ->
            order.getOrNull(old) ?: old
        }
    }
}
