package com.hsilighting.pagify.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.vector.ImageVector
import com.hsilighting.pagify.core.cloudOutline
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

/**
 * Letters on a curve.
 *
 * Drawn, like the curves it sits beside, because there is nothing in Material that
 * means "text that bends" — and because it has to read as a *pair* with the plain
 * text glyph next to it. An arc with three strokes standing on it: enough to say
 * "writing, following a line", at a size where actual letters would be a smudge.
 */
val CurvedTextIcon: ImageVector by lazy {
    ImageVector.Builder(
        name = "CurvedText",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        val ink = SolidColor(Color.Black)

        // The baseline the letters sit on.
        path(
            stroke = ink,
            strokeLineWidth = STROKE,
            strokeLineCap = StrokeCap.Round,
        ) {
            moveTo(2.5f, 16.5f)
            quadTo(12f, 6f, 21.5f, 16.5f)
        }

        // Three strokes standing off it, each square to the curve beneath — which
        // is the whole point of the tool and the only thing this glyph has to say.
        listOf(
            Triple(5.6f, 12.6f, -38f),
            Triple(12f, 9.6f, 0f),
            Triple(18.4f, 12.6f, 38f),
        ).forEach { (x, y, degrees) ->
            val radians = Math.toRadians(degrees.toDouble())
            val dx = (kotlin.math.sin(radians) * LETTER).toFloat()
            val dy = (kotlin.math.cos(radians) * LETTER).toFloat()
            path(
                stroke = ink,
                strokeLineWidth = STROKE,
                strokeLineCap = StrokeCap.Round,
            ) {
                moveTo(x, y)
                lineTo(x - dx, y - dy)
            }
        }
    }.build()
}

/** How tall the letters stand off the baseline, in the icon's own units. */
private const val LETTER = 5f

/**
 * Words inside a revision cloud.
 *
 * The ring is built by the same [cloudOutline] that draws the real thing, at
 * icon scale, so the glyph cannot drift away from the notation it stands for.
 */
val CloudTextIcon: ImageVector by lazy {
    ImageVector.Builder(
        name = "CloudText",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        val ink = SolidColor(Color.Black)
        val box = listOf(
            Offset(3.5f, 7.5f),
            Offset(20.5f, 7.5f),
            Offset(20.5f, 16.5f),
            Offset(3.5f, 16.5f),
        )
        val ring = cloudOutline(box, widthPoints = 0.7f)
        if (ring.size >= 2) {
            path(stroke = ink, strokeLineWidth = ICON_CLOUD_STROKE, strokeLineCap = StrokeCap.Round) {
                moveTo(ring.first().x, ring.first().y)
                ring.drop(1).forEach { lineTo(it.x, it.y) }
            }
        }

        // A capital T inside it: enough to say "words", and the one letter that
        // reads at this size.
        path(stroke = ink, strokeLineWidth = STROKE, strokeLineCap = StrokeCap.Round) {
            moveTo(9f, 10.2f)
            lineTo(15f, 10.2f)
        }
        path(stroke = ink, strokeLineWidth = STROKE, strokeLineCap = StrokeCap.Round) {
            moveTo(12f, 10.2f)
            lineTo(12f, 14.2f)
        }
    }.build()
}

/** Thinner than the letter strokes: the cloud is a border, not the subject. */
private const val ICON_CLOUD_STROKE = 1.2f

/** Words inside a box. */
val BoxTextIcon: ImageVector by lazy { framedTextIcon(round = false) }

/** Words inside an ellipse. */
val EllipseTextIcon: ImageVector by lazy { framedTextIcon(round = true) }

/**
 * A capital T inside a frame.
 *
 * The two share a builder because they differ in one thing — whether the frame is
 * a rectangle or an ellipse — and drawing the letter twice is how the two icons
 * would slowly stop matching.
 */
private fun framedTextIcon(round: Boolean): ImageVector = ImageVector.Builder(
    name = if (round) "EllipseText" else "BoxText",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    val ink = SolidColor(Color.Black)

    path(stroke = ink, strokeLineWidth = ICON_FRAME_STROKE) {
        if (round) {
            // Four arcs, because the builder has no ellipse of its own.
            moveTo(2.5f, 12f)
            arcToRelative(9.5f, 6.5f, 0f, true, true, 19f, 0f)
            arcToRelative(9.5f, 6.5f, 0f, true, true, -19f, 0f)
            close()
        } else {
            moveTo(2.5f, 5.5f)
            lineTo(21.5f, 5.5f)
            lineTo(21.5f, 18.5f)
            lineTo(2.5f, 18.5f)
            close()
        }
    }

    path(stroke = ink, strokeLineWidth = STROKE, strokeLineCap = StrokeCap.Round) {
        moveTo(9f, 9.8f)
        lineTo(15f, 9.8f)
    }
    path(stroke = ink, strokeLineWidth = STROKE, strokeLineCap = StrokeCap.Round) {
        moveTo(12f, 9.8f)
        lineTo(12f, 14.4f)
    }
}.build()

/** Thinner than the letter strokes: the frame is a border, not the subject. */
private const val ICON_FRAME_STROKE = 1.2f
