package com.hsilighting.pagify.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/**
 * A curved line, and the same curve with a head on it.
 *
 * Drawn rather than borrowed. Material's nearest offerings are a trend line and a
 * redo arrow, and both carry a meaning of their own — "this went up", "do that
 * again" — where these have to say only *this is the shape you will get*. The pair
 * also has to read as a pair, which two icons picked from a set will not.
 *
 * The same arc in both, so the head is the only difference between them: that is
 * the only difference between the tools.
 */
val CurvedLineIcon: ImageVector by lazy {
    curveIcon(withHead = false)
}

val CurvedArrowIcon: ImageVector by lazy {
    curveIcon(withHead = true)
}

private fun curveIcon(withHead: Boolean): ImageVector = ImageVector.Builder(
    name = if (withHead) "CurvedArrow" else "CurvedLine",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    val ink = SolidColor(Color.Black)

    // One sweep, rising to the right: the shape a curved line is reached for to
    // draw — round an obstacle, or from a note to the thing it is about.
    path(
        stroke = ink,
        strokeLineWidth = STROKE,
        strokeLineCap = StrokeCap.Round,
        strokeLineJoin = StrokeJoin.Round,
    ) {
        moveTo(3f, 17.5f)
        quadTo(9f, 4.5f, 20.5f, 7.5f)
    }

    if (withHead) {
        // Barbs opening back along the direction the curve is travelling when it
        // arrives — not along the straight line between the ends, which on a bend
        // this deep would sit visibly askew.
        path(
            stroke = ink,
            strokeLineWidth = STROKE,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
        ) {
            moveTo(14.6f, 8.9f)
            lineTo(20.5f, 7.5f)
            lineTo(16.2f, 3.3f)
        }
    }
}.build()

/** Matches the weight of the Material outline glyphs these sit beside. */
private const val STROKE = 1.9f
