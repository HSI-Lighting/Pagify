package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The JNI-to-Kotlin boundary for text runs.
 *
 * These were lost when `TextSegmentTest` was deleted: its geometry half was
 * genuinely superseded when `intersectsBand` moved out to [TextSelection], but the
 * parsing half covers `listFromJson`, which is unchanged and still the only path
 * text takes from the engine into the UI. Restored here rather than back into that
 * file, so the split matches what each one is actually about.
 */
class TextSegmentParseTest {

    @Test
    fun `dimensions derive from the edges`() {
        val subject = TextSegment(left = 10f, top = 20f, right = 110f, bottom = 50f, text = "x")
        assertEquals(100f, subject.width, 0f)
        assertEquals(30f, subject.height, 0f)
    }

    @Test
    fun `runs parse from the engine json`() {
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
    fun `a page with no text parses to an empty list`() {
        assertTrue(TextSegment.listFromJson("[]").isEmpty())
    }

    /**
     * The engine skips whitespace-only runs, so a segment with no `text` key
     * should not arise — but a parser that threw on one would turn a cosmetic
     * engine change into a crash while the user is selecting.
     */
    @Test
    fun `a run with no text field parses rather than throwing`() {
        val segments = TextSegment.listFromJson(
            """[{"left":1.0,"top":2.0,"right":3.0,"bottom":4.0}]""",
        )
        assertEquals(1, segments.size)
        assertEquals("", segments[0].text)
    }

    /**
     * The engine flips PDF's bottom-left origin once so nothing above it has to,
     * so top must always be the smaller number.
     */
    @Test
    fun `coordinates arrive top-left originated`() {
        val segments = TextSegment.listFromJson(
            """[{"left":10.0,"top":20.0,"right":110.0,"bottom":50.0,"text":"a"}]""",
        )
        assertTrue("top must be above bottom in screen space", segments[0].top < segments[0].bottom)
        assertTrue(segments[0].left < segments[0].right)
    }
}
