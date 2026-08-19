package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Selecting text over per-character geometry.
 *
 * The engine's side is covered against real PDFium in `text_selection.rs`; this
 * is the arithmetic on top — which character a finger landed on, which word that
 * belongs to, and what to paint. All of it is wrong in ways that look almost
 * right, which is why none of it lives inside a gesture handler.
 */
class PageCharactersTest {

    /**
     * Two lines of monospaced-by-construction text: every character 10 wide and
     * 12 tall, so an expected box can be written down rather than derived.
     *
     * "AB CD" on the first line, "EF" on the second.
     */
    private fun page(): PageCharacters {
        val text = "AB CD\nEF"
        val boxes = mutableListOf<Float>()
        var x = 0f
        var top = 0f
        for (character in text) {
            if (character == '\n') {
                // A newline has no glyph; the engine gives it a degenerate box.
                boxes += listOf(x, top, x, top)
                x = 0f
                top = 20f
                continue
            }
            boxes += listOf(x, top, x + 10f, top + 12f)
            x += 10f
        }
        return PageCharacters(text, boxes.toFloatArray())
    }

    @Test
    fun `a touch inside a character selects it`() {
        val page = page()
        assertEquals(0, page.indexNear(Offset(5f, 6f)))
        assertEquals(1, page.indexNear(Offset(15f, 6f)))
        assertEquals(6, page.indexNear(Offset(5f, 26f)))
    }

    @Test
    fun `a touch below the last line still lands on it`() {
        // Missing the text is the common case on a finger-sized target, and the
        // answer someone expects is the nearest character rather than nothing.
        val page = page()
        assertEquals(6, page.indexNear(Offset(5f, 60f)))
    }

    @Test
    fun `a touch between two lines prefers the nearer one`() {
        val page = page()
        // y = 14 is just below the first line (0..12) and well above the second
        // (20..32).
        assertEquals(0, page.indexNear(Offset(5f, 14f)))
        assertEquals(6, page.indexNear(Offset(5f, 19f)))
    }

    @Test
    fun `a page with no text has nothing to select`() {
        assertNull(PageCharacters.EMPTY.indexNear(Offset(5f, 5f)))
        assertEquals("", PageCharacters.EMPTY.textOf(0..3))
        assertEquals(emptyList<Rect>(), PageCharacters.EMPTY.rectsOf(0..3))
    }

    @Test
    fun `a long press selects the whole word, not one character`() {
        // Pointing at a letter means pointing at the word it is in. Starting from
        // one character would mean dragging a handle before anything useful is
        // selected.
        val page = page()
        assertEquals(0..1, page.wordAround(0))
        assertEquals(0..1, page.wordAround(1))
        assertEquals(3..4, page.wordAround(4))
    }

    @Test
    fun `a long press on a space selects only the space`() {
        // There is no word there to expand to, and swallowing a neighbouring one
        // would select something the finger was not on.
        val page = page()
        assertEquals(2..2, page.wordAround(2))
    }

    @Test
    fun `the selected text is exactly what the page holds`() {
        val page = page()
        assertEquals("AB", page.textOf(0..1))
        assertEquals("AB CD", page.textOf(0..4))
        // Across the line break, newline included: what is copied should paste
        // back as two lines.
        assertEquals("AB CD\nEF", page.textOf(0..7))
    }

    @Test
    fun `a range running past the end is clamped rather than throwing`() {
        val page = page()
        assertEquals("EF", page.textOf(6..99))
    }

    @Test
    fun `characters on one line are painted as one band`() {
        val page = page()
        val rects = page.rectsOf(0..4)

        assertEquals(1, rects.size)
        assertEquals(Rect(0f, 0f, 50f, 12f), rects[0])
    }

    @Test
    fun `a selection across lines is painted as one band per line`() {
        val page = page()
        val rects = page.rectsOf(0..7)

        assertEquals(2, rects.size)
        assertEquals(Rect(0f, 0f, 50f, 12f), rects[0])
        assertEquals(Rect(0f, 20f, 20f, 32f), rects[1])
    }

    @Test
    fun `a gap wide enough to be another column is not painted through`() {
        // Two columns can share a line. Painting across the gutter would claim
        // text that was never selected — the same mistake the run-based
        // highlighter made before it was taught about reading order.
        val text = "AB"
        val boxes = floatArrayOf(
            0f, 0f, 10f, 12f,
            // Second character 60 units further on: five line heights of gap.
            70f, 0f, 80f, 12f,
        )
        val rects = PageCharacters(text, boxes).rectsOf(0..1)

        assertEquals(2, rects.size)
        assertEquals(Rect(0f, 0f, 10f, 12f), rects[0])
        assertEquals(Rect(70f, 0f, 80f, 12f), rects[1])
    }

    @Test
    fun `an ordinary space does not break the band`() {
        // The gap between two words is well under a line height, and breaking
        // there would draw a separate rectangle for every word.
        val page = page()
        assertEquals(1, page.rectsOf(0..4).size)
    }

    @Test
    fun `the newline itself paints nothing`() {
        val page = page()
        // Selecting just the line break gives an empty band rather than a
        // zero-width sliver at the end of the line.
        assertEquals(emptyList<Rect>(), page.rectsOf(5..5))
    }

    @Test
    fun `the wire form is what the engine sends`() {
        // Pinned as a literal on both sides: the alignment between the text and
        // the boxes is the whole contract, and a field name that does not match
        // fails on a device and nowhere else.
        val json = """{"text":"Hi","boxes":[0.0,1.0,10.0,13.0,10.0,1.0,20.0,13.0]}"""
        val page = PageCharacters.fromJson(json)

        assertEquals("Hi", page.text)
        assertEquals(2, page.count)
        assertEquals(Rect(10f, 1f, 20f, 13f), page.boxAt(1))
    }

    @Test
    fun `every character has a box`() {
        // The one invariant the engine promises. Asserted here as well, because
        // everything above indexes one against the other.
        val page = page()
        assertTrue(page.boxes.size == page.text.length * 4)
    }
}
