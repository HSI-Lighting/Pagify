package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Words drawn on a screenshot.
 *
 * The one thing that makes these different from words on a page: a picture has no
 * text layer, so a framed caption has to come apart on the way out — a stroked
 * ring and a filled set of letters, because no single drawing operation is both.
 * Everywhere else it stays one mark, and these pin that down.
 */
class MarkupTextTest {

    private fun caption(
        words: String = "Check this",
        frame: TextFrame = TextFrame.None,
        at: Offset = Offset(40f, 90f),
    ) = Markup(
        shape = MarkupShape.Text(
            text = words,
            path = listOf(at),
            font = PdfFont.HELVETICA,
            sizePoints = 12f,
            frame = frame,
        ),
        color = AnnotationColors.RED,
    )

    @Test
    fun `plain words go out as one mark`() {
        assertEquals(1, caption().forWire().size)
    }

    @Test
    fun `framed words go out as the ring and then the letters`() {
        val out = caption(frame = TextFrame.Cloud).forWire()

        assertEquals(2, out.size)
        // The ring first: the letters are drawn over it where the two meet, and
        // the engine paints in the order it is given.
        assertTrue(out[0].shape is MarkupShape.Freehand)
        assertTrue(out[1].shape is MarkupShape.Text)
        assertTrue((out[0].shape as MarkupShape.Freehand).points.size > 8)
    }

    @Test
    fun `the ring is drawn in the colour of the words`() {
        val mark = caption(frame = TextFrame.Box)
        assertEquals(mark.color, mark.forWire()[0].color)
    }

    @Test
    fun `the ring goes round the words, not through them`() {
        val words = caption(frame = TextFrame.Box).shape as MarkupShape.Text
        val ring = (caption(frame = TextFrame.Box).forWire()[0].shape as MarkupShape.Freehand)

        val anchor = words.path.first()
        val end = anchor.x + words.font.widthOf(words.text, words.sizePoints)
        assertTrue("the ring starts right of the first letter", ring.points.minOf { it.x } < anchor.x)
        assertTrue("the ring ends left of the last letter", ring.points.maxOf { it.x } > end)
    }

    @Test
    fun `a caption is grabbable along its whole run`() {
        val words = caption().shape as MarkupShape.Text
        val end = 40f + words.font.widthOf(words.text, words.sizePoints)

        assertTrue(words.isHitBy(Offset(41f, 90f), tolerance = 1f))
        assertTrue(words.isHitBy(Offset(end - 2f, 90f), tolerance = 1f))
        assertFalse(words.isHitBy(Offset(end + 80f, 90f), tolerance = 1f))
    }

    @Test
    fun `moving a caption takes its frame with it`() {
        val moved = (caption(frame = TextFrame.Ellipse).shape as MarkupShape.Text)
            .movedBy(Offset(25f, -10f))

        assertEquals(Offset(65f, 80f), moved.path.single())
        // The frame is measured from the words rather than stored, so it cannot
        // be left behind: this is the property that makes it one mark.
        assertEquals(TextFrame.Ellipse, moved.frame)
    }

    @Test
    fun `blank words are not a mark`() {
        assertFalse((caption(words = "  ").shape as MarkupShape.Text).isBigEnough())
        assertTrue((caption().shape as MarkupShape.Text).isBigEnough())
    }
}
