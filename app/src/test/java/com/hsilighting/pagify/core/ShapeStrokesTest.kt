package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.hypot

/**
 * Turning a drag into the strokes that get written into the PDF.
 *
 * These are the parts that are wrong in ways that look plausible: an arrow head
 * on the wrong end, an ellipse that is really a circle, a dashed line whose
 * dashes drift as it crosses a corner. All pure, so none of it needs a device.
 */
class ShapeStrokesTest {

    private val from = Offset(100f, 100f)
    private val to = Offset(300f, 100f)

    private fun length(stroke: List<Offset>): Float =
        stroke.zipWithNext().sumOf { (a, b) -> hypot(b.x - a.x, b.y - a.y).toDouble() }.toFloat()

    private fun inked(strokes: List<List<Offset>>): Float = strokes.sumOf { length(it).toDouble() }
        .toFloat()

    @Test
    fun `a line is one stroke from end to end`() {
        val strokes = shapeStrokes(AnnotationTool.Line, from, to, MarkupStyle.SOLID, 2f)

        assertEquals(1, strokes.size)
        assertEquals(listOf(from, to), strokes.single())
    }

    @Test
    fun `an arrow is a shaft and two barbs`() {
        val strokes = shapeStrokes(AnnotationTool.Arrow, from, to, MarkupStyle.SOLID, 2f)

        assertEquals(3, strokes.size)
        assertEquals(listOf(from, to), strokes.first())
        // Both barbs start at the tip — that is what keeps the point sharp.
        assertTrue(strokes.drop(1).all { it.first() == to })
        // And both point back down the shaft rather than past the tip.
        assertTrue(strokes.drop(1).all { it.last().x < to.x })
    }

    @Test
    fun `the arrow head is at the end that was dragged to`() {
        // The mistake this catches is a sign flip, which looks fine until someone
        // points an arrow at something and it points away.
        val backwards = shapeStrokes(AnnotationTool.Arrow, to, from, MarkupStyle.SOLID, 2f)

        assertTrue(backwards.drop(1).all { it.first() == from })
        assertTrue(backwards.drop(1).all { it.last().x > from.x })
    }

    @Test
    fun `a rectangle closes`() {
        val strokes = shapeStrokes(
            AnnotationTool.Rectangle,
            Offset(10f, 20f),
            Offset(50f, 60f),
            MarkupStyle.SOLID,
            2f,
        )
        val outline = strokes.single()

        assertEquals(5, outline.size)
        assertEquals(outline.first(), outline.last())
        assertEquals(160f, length(outline), 0.01f)
    }

    @Test
    fun `a rectangle dragged up and to the left is the same rectangle`() {
        val forward = shapeStrokes(
            AnnotationTool.Rectangle,
            Offset(10f, 20f),
            Offset(50f, 60f),
            MarkupStyle.SOLID,
            2f,
        )
        val backward = shapeStrokes(
            AnnotationTool.Rectangle,
            Offset(50f, 60f),
            Offset(10f, 20f),
            MarkupStyle.SOLID,
            2f,
        )

        assertEquals(forward, backward)
    }

    @Test
    fun `an ellipse fills the rectangle it was dragged in`() {
        val strokes = shapeStrokes(
            AnnotationTool.Ellipse,
            Offset(0f, 0f),
            Offset(200f, 100f),
            MarkupStyle.SOLID,
            2f,
        )
        val outline = strokes.single()

        // Touching each edge, and no wider: an ellipse that came out as a circle
        // would miss two of these by fifty points.
        assertEquals(0f, outline.minOf { it.x }, 0.5f)
        assertEquals(200f, outline.maxOf { it.x }, 0.5f)
        assertEquals(0f, outline.minOf { it.y }, 0.5f)
        assertEquals(100f, outline.maxOf { it.y }, 0.5f)
        assertEquals(outline.first(), outline.last())
    }

    @Test
    fun `a solid line is not cut up`() {
        // The common case pays nothing: one stroke in, one stroke out.
        val strokes = dashed(listOf(from, to), MarkupStyle.SOLID, 2f)
        assertEquals(listOf(listOf(from, to)), strokes)
    }

    @Test
    fun `a dashed line is cut into pieces with gaps between them`() {
        val strokes = dashed(listOf(from, to), MarkupStyle.DASH_1, 2f)

        assertTrue("only ${strokes.size} pieces", strokes.size > 3)
        // Ink covers the dashes and not the gaps: 8 points on, 6 off, so a little
        // over half of the 200-point line.
        val covered = inked(strokes) / length(listOf(from, to))
        assertTrue("covered $covered", covered > 0.4f && covered < 0.75f)
    }

    @Test
    fun `every dash lies on the line it came from`() {
        // The failure this catches is a dash that walks off the line as it goes,
        // which happens the moment the remaining length is measured from the wrong
        // point.
        val strokes = dashed(listOf(from, to), MarkupStyle.DASH_2, 3f)

        strokes.flatten().forEach { point ->
            assertEquals("off the line at $point", 100f, point.y, 0.01f)
            assertTrue("past the ends at $point", point.x >= 99.9f && point.x <= 300.1f)
        }
    }

    @Test
    fun `dashes carry on around a corner`() {
        // A rectangle is four runs in one polyline. Dashing each segment
        // separately would restart the pattern at every corner and put a dash on
        // each — this checks the walk is by length, not by point.
        val square = listOf(
            Offset(0f, 0f),
            Offset(100f, 0f),
            Offset(100f, 100f),
            Offset(0f, 100f),
            Offset(0f, 0f),
        )
        val strokes = dashed(square, MarkupStyle.DASH_1, 2f)

        val corners = strokes.count { stroke ->
            stroke.zipWithNext().any { (a, b) -> abs(a.x - b.x) > 0.01f && abs(a.y - b.y) > 0.01f }
        }
        assertEquals("a dash cut the corner", 0, corners)
        assertTrue("only ${strokes.size} dashes on 400 points", strokes.size > 20)
    }

    @Test
    fun `a centre line has short pieces among its long ones`() {
        val strokes = dashed(listOf(from, to), MarkupStyle.CENTERLINE_1, 2f)
        val longest = strokes.maxOf { length(it) }

        assertTrue(
            "no dots among ${strokes.map { length(it) }}",
            strokes.any { length(it) * 3f < longest },
        )
    }

    @Test
    fun `the pen and the eraser have no shape to draw`() {
        listOf(AnnotationTool.Pen, AnnotationTool.Eraser, AnnotationTool.None).forEach { tool ->
            assertEquals(
                "$tool produced strokes",
                emptyList<List<Offset>>(),
                shapeStrokes(tool, from, to, MarkupStyle.SOLID, 2f),
            )
        }
    }
}
