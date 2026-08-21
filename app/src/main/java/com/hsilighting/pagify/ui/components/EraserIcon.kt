package com.hsilighting.pagify.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/**
 * An eraser, drawn rather than borrowed.
 *
 * Material's icon set has no eraser. The nearest thing anyone reaches for is
 * `Backspace`, which is what this tool used — and a backspace arrow means "delete
 * the character behind the cursor" to everybody who has ever used a keyboard, so
 * the tool read as a text control rather than as a rubber.
 *
 * Traced from the reference the shape came from: a long block laid over at about
 * forty degrees, rounded at the rubber end, a ferrule across it a third of the way
 * up, and the paper it is being dragged along running off to the right.
 *
 * The ferrule is what makes it an eraser rather than a tilted box — without it the
 * glyph is a parallelogram and reads as nothing at all.
 *
 * Stroked rather than filled, to sit beside the other outline glyphs in the
 * ribbon. `Icon` tints the whole painter, so the stroke takes the tint like any
 * other icon.
 */
val EraserIcon: ImageVector by lazy {
    ImageVector.Builder(
        name = "Eraser",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        val ink = SolidColor(Color.Black)

        // The block. Round joins do the rounding at the rubber end, which is the
        // corner the eye actually reads as "rubber" rather than "wedge".
        path(
            stroke = ink,
            strokeLineWidth = STROKE,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
        ) {
            moveTo(2.7f, 14.2f)
            lineTo(16.3f, 2.4f)
            lineTo(21.3f, 8.2f)
            lineTo(7.7f, 20.0f)
            close()
        }

        // The ferrule, across the block a third of the way up: rubber below the
        // line, holder above it.
        path(
            stroke = ink,
            strokeLineWidth = STROKE,
            strokeLineCap = StrokeCap.Round,
        ) {
            moveTo(7.5f, 9.4f)
            lineTo(12.5f, 15.2f)
        }

        // The paper, running off to the right from where the block meets it.
        path(
            stroke = ink,
            strokeLineWidth = STROKE,
            strokeLineCap = StrokeCap.Round,
        ) {
            moveTo(7.7f, 20.0f)
            lineTo(21.3f, 20.0f)
        }
    }.build()
}

/** Matches the weight of the Material outline glyphs it sits beside. */
private const val STROKE = 1.9f
