package com.hsilighting.pagify.core

import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PathMeasure
import android.graphics.Typeface
import androidx.compose.ui.geometry.Offset

/**
 * Letters as the outlines they are made of.
 *
 * A capture is a picture. It has no text layer, so words drawn on one cannot be
 * written as text the way they are on a page — they have to become shapes. The
 * platform knows the shapes, since it is the thing that drew them on screen, and
 * [Paint.getTextPath] hands them over; this walks the result into plain polylines
 * the engine can fill.
 *
 * Sampled rather than sent as curves because the wire carries points, and the
 * engine fills polygons. The step is a fraction of the point size, so a letter is
 * sampled as finely at nine points as at seventy-two.
 */
fun MarkupShape.Text.glyphContours(): List<List<Offset>> {
    if (text.isBlank()) return emptyList()

    val paint = Paint().apply {
        isAntiAlias = true
        textSize = sizePoints
        typeface = Typeface.create(
            font.family,
            if (font.bold) Typeface.BOLD else Typeface.NORMAL,
        )
    }

    val letters = Path()
    layOutBlock().forEach { placement ->
        val glyph = Path()
        val letter = placement.character.toString()
        paint.getTextPath(letter, 0, letter.length, 0f, 0f, glyph)
        // Rotated about its own origin and then carried to it, which is the order
        // that puts a letter on a curve. The other way round rotates the whole
        // line about the page's corner and scatters the word across the picture.
        val place = Matrix().apply {
            setRotate(Math.toDegrees(placement.radians.toDouble()).toFloat())
            postTranslate(placement.origin.x, placement.origin.y)
        }
        glyph.transform(place)
        letters.addPath(glyph)
    }

    return letters.contours(step = (sizePoints * SAMPLE_FRACTION).coerceAtLeast(MINIMUM_STEP))
}

/**
 * Every contour of this path, sampled into points at most [step] apart.
 *
 * Contour by contour rather than over the path as a whole: the hole in an "o" is
 * a contour of its own, and running the two together joins them with a line
 * straight through the letter.
 */
private fun Path.contours(step: Float): List<List<Offset>> {
    val measure = PathMeasure(this, false)
    val contours = mutableListOf<List<Offset>>()
    val position = FloatArray(2)

    do {
        val length = measure.length
        if (length <= 0f) continue
        val count = (length / step).toInt().coerceIn(3, MAXIMUM_SAMPLES)
        val points = ArrayList<Offset>(count)
        for (index in 0 until count) {
            val at = length * index / count
            if (measure.getPosTan(at, position, null)) {
                points += Offset(position[0], position[1])
            }
        }
        if (points.size >= 3) contours += points
    } while (measure.nextContour())

    return contours
}

/** How finely a letter is sampled, as a fraction of its point size. */
private const val SAMPLE_FRACTION = 0.03f

/** Below this the sampling stops buying anything a viewer can see. */
private const val MINIMUM_STEP = 0.05f

/**
 * A ceiling on the points in one contour.
 *
 * Not a quality setting — at the sampling above nothing real comes close to it.
 * It is there so that a pathological glyph cannot put a megabyte of coordinates
 * on the wire.
 */
private const val MAXIMUM_SAMPLES = 512
