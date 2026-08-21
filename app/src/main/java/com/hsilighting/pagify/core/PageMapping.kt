package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect

/**
 * Where a point on the page lands on screen, and back again.
 *
 * Marks are stored in page points, which is what lets one stroke survive a zoom,
 * a rotation and a reopen. Everything that draws or hit-tests one has to convert,
 * and until the reader could rotate that conversion was a multiply and an add —
 * so it was written inline, twice per call site, in about twenty places.
 *
 * Rotation is what makes it worth a type. A turned page is not a scaled page: the
 * axes swap and one of them flips, and getting the flip backwards puts every mark
 * on the wrong side of the sheet in a way that still looks like a page of marks.
 * Pure, and here rather than in the layer that draws, because that is the only way
 * to test it without a device.
 *
 * @param scale pixels per point, against the page *as laid out* — so against its
 *   turned width, which is what the box on screen is measured in.
 * @param origin top-left of the drawn page, in the space touches arrive in.
 * @param quarterTurns view rotation, clockwise, matching the renderer's.
 * @param pageWidthPoints the page's own width, before any turn.
 * @param pageHeightPoints likewise its own height.
 */
data class PageMapping(
    val scale: Float,
    val origin: Offset = Offset.Zero,
    val quarterTurns: Int = 0,
    val pageWidthPoints: Float = 0f,
    val pageHeightPoints: Float = 0f,
) {

    /** Whether anything can be placed at all; a page not yet measured cannot. */
    val isUsable: Boolean get() = scale > 0f

    private val turns: Int get() = ((quarterTurns % 4) + 4) % 4

    /** A page point, in the pixels of the page as drawn. */
    fun toScreen(point: Offset): Offset = turnPoint(point) * scale + origin

    /** A touch, in the page's own points. */
    fun toPage(position: Offset): Offset {
        if (!isUsable) return Offset.Zero
        return unturnPoint((position - origin) / scale)
    }

    /**
     * A rectangle of page points, as the box it covers on screen.
     *
     * Both corners go through the turn and the result is normalised, because a
     * quarter turn sends a top-left corner to a top-right one — keeping the
     * corners in their original roles would give a rectangle of negative width,
     * which draws as nothing at all.
     */
    fun toScreen(rect: Rect): Rect {
        val a = toScreen(Offset(rect.left, rect.top))
        val b = toScreen(Offset(rect.right, rect.bottom))
        return Rect(
            left = minOf(a.x, b.x),
            top = minOf(a.y, b.y),
            right = maxOf(a.x, b.x),
            bottom = maxOf(a.y, b.y),
        )
    }

    /** A length in points, in pixels. Rotation does not change a length. */
    fun toScreen(lengthPoints: Float): Float = lengthPoints * scale

    /** A length in pixels, in page points. */
    fun toPage(lengthPixels: Float): Float = if (isUsable) lengthPixels / scale else 0f

    /**
     * A page point in the turned page's own point space.
     *
     * Clockwise, to match `PdfPageRenderRotation` in the engine: after a quarter
     * turn the page's top-left corner is at the top *right*, so x comes from the
     * height and y comes from x.
     */
    private fun turnPoint(point: Offset): Offset = when (turns) {
        1 -> Offset(pageHeightPoints - point.y, point.x)
        2 -> Offset(pageWidthPoints - point.x, pageHeightPoints - point.y)
        3 -> Offset(point.y, pageWidthPoints - point.x)
        else -> point
    }

    /** The inverse of [turnPoint]. */
    private fun unturnPoint(point: Offset): Offset = when (turns) {
        1 -> Offset(point.y, pageHeightPoints - point.x)
        2 -> Offset(pageWidthPoints - point.x, pageHeightPoints - point.y)
        3 -> Offset(pageWidthPoints - point.y, point.x)
        else -> point
    }

    companion object {
        /** Nothing measured yet: every conversion is a no-op the caller can skip. */
        val Unmeasured = PageMapping(scale = 0f)
    }
}
