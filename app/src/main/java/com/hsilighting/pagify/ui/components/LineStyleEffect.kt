package com.hsilighting.pagify.ui.components

import androidx.compose.ui.graphics.PathEffect
import com.hsilighting.pagify.core.MarkupStyle

/**
 * The dash pattern to draw a mark with on screen.
 *
 * **Mirrors `StrokeStyle::dash` in the engine, and has to.** What is on screen is
 * a preview of what the export will contain: a line that dashes one way while it
 * is being drawn and another way in the file is a small lie told at exactly the
 * moment someone is deciding whether the mark is right.
 *
 * Proportional to the stroke width for the same reason it is there — a heavy
 * dashed line should read as the same kind of line as a fine one — and round caps
 * are what turn the near-zero segment in a centre line into a dot.
 */
fun MarkupStyle.pathEffect(widthPx: Float): PathEffect? {
    val w = widthPx.coerceAtLeast(1f)
    val dot = w * DOT_LENGTH
    val pattern = when (this) {
        MarkupStyle.SOLID -> return null
        MarkupStyle.DASH_1 -> floatArrayOf(w * 4f, w * 3f)
        MarkupStyle.DASH_2 -> floatArrayOf(w * 9f, w * 4f)
        MarkupStyle.CENTERLINE_1 -> floatArrayOf(w * 9f, w * 3f, dot, w * 3f)
        MarkupStyle.CENTERLINE_2 -> floatArrayOf(w * 9f, w * 3f, dot, w * 3f, dot, w * 3f)
    }
    return PathEffect.dashPathEffect(pattern, 0f)
}

/** How long a dot is, as a fraction of the stroke width. */
private const val DOT_LENGTH = 0.01f
