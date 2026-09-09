package com.hsilighting.pagify.ui.components

import androidx.compose.ui.geometry.Offset
import com.hsilighting.pagify.core.MAXIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.MINIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.curvedBaseline
import com.hsilighting.pagify.core.layOutBlock
import com.hsilighting.pagify.core.sizeRange
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The marks on a capture, and the rules about them.
 *
 * Pure state, so all of it can be checked without a device — which matters
 * because the ways this goes wrong are silent: a caption that will not grow
 * reads as a broken gesture, not as a number clamped by the wrong range.
 */
class CaptureMarkupTest {

    private fun caption(points: Float = 14f, text: String = "NOTE") = MarkupShape.Text(
        text = text,
        path = listOf(Offset(10f, 10f)),
        font = PdfFont.HELVETICA,
        sizePoints = points,
    )

    private fun withCaption(points: Float = 14f): CaptureMarkup {
        val markup = CaptureMarkup()
        markup.use(MarkupTool.Text)
        markup.add(caption(points))
        markup.select(0)
        return markup
    }

    /**
     * **A pinch scales a caption in points, not in nib widths.**
     *
     * The two ranges are nothing like each other: a pen's width runs 0.6 to 16
     * and a caption's size 6 to 400. Clamping one by the other pins every
     * caption at sixteen points the instant it is pinched — and what somebody
     * sees is a gesture that does not work, beside a size bar that plainly
     * does, which is a very hard thing to guess the cause of.
     */
    @Test
    fun `a caption can be pinched past a pen's widest nib`() {
        val markup = withCaption(points = 14f)

        markup.scaleSelected(4f)

        val grown = (markup.marks[0].shape as MarkupShape.Text).sizePoints
        assertTrue("stuck at $grown", grown > MarkupTool.Text.sizeRange.endInclusive)
        assertEquals(56f, grown, 0.01f)
    }

    /** And the bar follows it, so the two never disagree. */
    @Test
    fun `the size bar follows a pinch`() {
        val markup = withCaption(points = 14f)

        markup.scaleSelected(2f)

        assertEquals(28f, markup.textSizePoints, 0.01f)
    }

    /** It stops at the ends of the range rather than running away. */
    @Test
    fun `a caption stays within the sizes text can be`() {
        val huge = withCaption(points = 300f)
        huge.scaleSelected(10f)
        assertTrue(
            (huge.marks[0].shape as MarkupShape.Text).sizePoints <= MAXIMUM_TEXT_POINTS,
        )

        val tiny = withCaption(points = 7f)
        tiny.scaleSelected(0.01f)
        assertTrue(
            (tiny.marks[0].shape as MarkupShape.Text).sizePoints >= MINIMUM_TEXT_POINTS,
        )
    }

    /**
     * A caption cannot be grown wider than the picture it is written on.
     *
     * Given the picture's width, the ceiling is what fits across it; without
     * one the words run off the edge, where the last of them cannot be read
     * and cannot be brought back except by shrinking blind.
     */
    @Test
    fun `a caption cannot be grown off the edge of the picture`() {
        val markup = withCaption(points = 14f)

        markup.scaleSelected(50f, across = 400f)

        val grown = (markup.marks[0].shape as MarkupShape.Text).sizePoints
        assertTrue("$grown is wider than the picture", grown < MAXIMUM_TEXT_POINTS)
    }

    /** Nothing selected, nothing scaled. */
    @Test
    fun `a pinch with nothing in hand changes nothing`() {
        val markup = CaptureMarkup()
        markup.use(MarkupTool.Text)
        markup.add(caption())

        markup.scaleSelected(3f)

        assertEquals(14f, (markup.marks[0].shape as MarkupShape.Text).sizePoints, 0.01f)
    }

    /**
     * Clearing a caption's words is how one is deleted.
     *
     * There is no second way to remove it, so if this stopped working a
     * caption could be emptied and would stay on the picture as nothing.
     */
    @Test
    fun `emptying a caption removes it`() {
        val markup = withCaption()

        markup.rewrite(0, "   ")

        assertTrue(markup.marks.isEmpty())
        assertNull(markup.selected)
    }

    /** Undo takes the last mark off, and lets go of anything held. */
    @Test
    fun `undo removes the last mark`() {
        val markup = withCaption()
        markup.add(caption(text = "SECOND"))

        markup.undo()

        assertEquals(1, markup.marks.size)
        assertNull(markup.selected)
    }

    /** Each tool keeps its own weight, so switching does not reset the other. */
    @Test
    fun `every tool remembers how heavy it draws`() {
        val markup = CaptureMarkup()

        markup.size(MarkupTool.Pen, 3f)
        markup.size(MarkupTool.Highlight, 0.5f)

        markup.use(MarkupTool.Pen)
        assertEquals(3f, markup.size, 0.01f)
        markup.use(MarkupTool.Highlight)
        assertEquals(0.5f, markup.size, 0.01f)
    }

    /** Picking a tool up arms it; putting it down keeps which one it was. */
    @Test
    fun `a tool put down is still the tool`() {
        val markup = CaptureMarkup()

        markup.use(MarkupTool.Cloud)
        markup.disarm()

        assertEquals(MarkupTool.Cloud, markup.tool)
        assertTrue(!markup.armed)
    }

    /**
     * A caption written on a real baseline, the way the editor makes one.
     *
     * The other helper here places a caption at a single point, which never
     * lays out at all — enough for the state rules above, and no use for the
     * ones below, which are all about the line the words are written on.
     */
    private fun written(words: String, points: Float): CaptureMarkup {
        val markup = CaptureMarkup()
        markup.use(MarkupTool.Text)
        markup.add(
            MarkupShape.Text(
                text = words,
                path = curvedBaseline(Offset(10f, 200f), words, PdfFont.HELVETICA, points, 0f),
                font = PdfFont.HELVETICA,
                sizePoints = points,
            ),
        )
        markup.select(0)
        return markup
    }

    private fun laidOut(markup: CaptureMarkup): Int =
        (markup.marks[0].shape as MarkupShape.Text).layOutBlock().size

    /**
     * **Growing a caption does not cost it letters.**
     *
     * A caption is words and the line they are written on, and the layout walks
     * that line and stops where it ends. Changing the size without rebuilding
     * the line leaves the words too long for it, and the ones past the end are
     * not drawn at all: growing a caption ate it from the right, one letter at
     * a time, and shrinking it fed them back. That reads as the text being
     * eaten rather than as a line that never grew — which is why no amount of
     * care with the gesture helped.
     */
    @Test
    fun `a caption keeps every letter as it grows`() {
        val words = "SECTION THROUGH STAIR"
        val markup = written(words, 20f)
        assertEquals("the baseline was too short to begin with", words.length, laidOut(markup))

        markup.scaleSelected(4f)

        assertEquals(words.length, laidOut(markup))
    }

    /** The size bar is the same story, and was the same `copy`. */
    @Test
    fun `the size bar keeps every letter too`() {
        val words = "TOP OF SECOND FLR"
        val markup = written(words, 20f)

        markup.textSize(180f)

        assertEquals(words.length, laidOut(markup))
    }

    /** So is changing the face: a wider one needs more line. */
    @Test
    fun `a wider face keeps every letter`() {
        val words = "ELEV 11-9"
        val markup = written(words, 40f)

        markup.font(PdfFont.TIMES_BOLD)

        assertEquals(words.length, laidOut(markup))
    }

    /**
     * A caption starts big enough to read on the picture it is written on.
     *
     * The reader's fourteen points is body text on a sheet six hundred points
     * across; a capture is measured in its own pixels, thousands of them, and
     * the same number came out as a line a fiftieth of the width.
     */
    @Test
    fun `a caption starts at the size a picture needs`() {
        assertEquals(CAPTION_POINTS, CaptureMarkup().textSizePoints, 0.01f)
        assertTrue("too small to read on a picture", CaptureMarkup().textSizePoints >= 100f)
    }
}
