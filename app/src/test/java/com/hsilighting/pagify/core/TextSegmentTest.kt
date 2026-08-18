package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Text selection is about to get a UI, and its geometry is pure arithmetic —
 * exactly the kind of thing that should never need a device to verify.
 *
 * These deliberately mirror the Rust tests for `TextSegment::intersects_band`:
 * both sides implement the same rule, and the two can drift independently.
 */
class TextSegmentTest {

    private fun line(top: Float, bottom: Float, text: String = "run") =
        TextSegment(left = 10f, top = top, right = 200f, bottom = bottom, text = text)

    @Test
    fun aDragTakesEveryLineItCrossesAndNoneBeyond() {
        val line1 = line(100f, 120f)
        val line2 = line(130f, 150f)
        val line3 = line(160f, 180f)
        val line4 = line(190f, 210f)

        val from = 110f
        val to = 170f
        assertTrue(line1.intersectsBand(from, to))
        assertTrue("a line fully inside the band is taken whole", line2.intersectsBand(from, to))
        assertTrue(line3.intersectsBand(from, to))
        assertFalse("line four sits past the release point", line4.intersectsBand(from, to))
    }

    @Test
    fun selectionDoesNotDependOnDragDirection() {
        val subject = line(130f, 150f)
        for ((a, b) in listOf(110f to 170f, 140f to 140f, 90f to 135f, 145f to 400f)) {
            assertEquals(
                "dragging $a to $b and $b to $a must agree",
                subject.intersectsBand(a, b),
                subject.intersectsBand(b, a),
            )
        }
    }

    @Test
    fun aBandThatOnlyGrazesALineStillTakesIt() {
        val subject = line(130f, 150f)
        assertTrue("touching the bottom edge counts", subject.intersectsBand(150f, 160f))
        assertFalse("one point past it does not", subject.intersectsBand(151f, 160f))
        assertTrue("touching the top edge counts", subject.intersectsBand(120f, 130f))
        assertFalse(subject.intersectsBand(120f, 129f))
    }

    @Test
    fun dimensionsDeriveFromTheEdges() {
        val subject = TextSegment(left = 10f, top = 20f, right = 110f, bottom = 50f, text = "x")
        assertEquals(100f, subject.width, 0f)
        assertEquals(30f, subject.height, 0f)
    }

    // -------------------------------------------------------------- parsing --

    @Test
    fun runsParseFromTheEngineJson() {
        val json = """
            [
              {"left":10.5,"top":20.25,"right":110.5,"bottom":50.0,"text":"Hello"},
              {"left":10.5,"top":60.0,"right":90.0,"bottom":85.0,"text":"world"}
            ]
        """.trimIndent()

        val segments = TextSegment.listFromJson(json)

        assertEquals(2, segments.size)
        assertEquals(TextSegment(10.5f, 20.25f, 110.5f, 50f, "Hello"), segments[0])
        assertEquals("world", segments[1].text)
    }

    @Test
    fun aPageWithNoTextParsesToAnEmptyList() {
        assertTrue(TextSegment.listFromJson("[]").isEmpty())
    }

    /**
     * The engine skips whitespace-only runs, so a segment with no `text` key
     * should not arise — but a parser that threw on one would turn a cosmetic
     * engine change into a crash while the user is selecting.
     */
    @Test
    fun aRunWithNoTextFieldParsesRatherThanThrowing() {
        val segments = TextSegment.listFromJson(
            """[{"left":1.0,"top":2.0,"right":3.0,"bottom":4.0}]""",
        )
        assertEquals(1, segments.size)
        assertEquals("", segments[0].text)
    }

    /**
     * The engine flips PDF bottom-left origin once so nothing above it has to,
     * so top must always be the smaller number.
     */
    @Test
    fun coordinatesArriveTopLeftOriginated() {
        val segments = TextSegment.listFromJson(
            """[{"left":10.0,"top":20.0,"right":110.0,"bottom":50.0,"text":"a"}]""",
        )
        assertTrue("top must be above bottom in screen space", segments[0].top < segments[0].bottom)
        assertTrue(segments[0].left < segments[0].right)
    }
}
