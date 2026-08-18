package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Selection is checked against runs extracted from the real document that broke
 * it — page 145 of the HSI catalogue, a two-column spread.
 *
 * The fixture is regenerated with
 * `cargo run --example inspect_text -- <pdf> 144 tsv`, so these tests describe
 * what PDFium actually reports rather than what a hand-written fixture makes
 * convenient. On this page the left column occupies x 194-375 and the right
 * column x 386-569, and their first lines share a y band within a tenth of a
 * point — which is precisely why a band-based selection could not tell them
 * apart.
 */
class TextSelectionTest {

    private val page: List<TextSegment> by lazy {
        val stream = checkNotNull(javaClass.getResourceAsStream(FIXTURE)) {
            "missing fixture $FIXTURE"
        }
        stream.bufferedReader().useLines { lines ->
            lines.filter { it.isNotBlank() }.map { line ->
                val f = line.split('\t', limit = 5)
                TextSegment(
                    left = f[0].toFloat(),
                    top = f[1].toFloat(),
                    right = f[2].toFloat(),
                    bottom = f[3].toFloat(),
                    text = f.getOrElse(4) { "" },
                )
            }.toList()
        }
    }

    @Test
    fun `fixture is the page that was reported`() {
        assertEquals(469, page.size)
        assertTrue(page.any { it.text.startsWith("The outdoor LED luminaire") })
    }

    @Test
    fun `dragging along one line stays on that line`() {
        val rects = TextSelection.rectsBetween(
            page,
            anchor = Offset(250f, 72f),
            focus = Offset(330f, 72f),
        )
        assertEquals(1, rects.size)
        assertEquals(250f, rects[0].left, 0.01f)
        assertEquals(330f, rects[0].right, 0.01f)
    }

    @Test
    fun `dragging down the left column never reaches the right one`() {
        // From the first line of the description to "where long-distance
        // lighting is required." sixteen lines below it.
        val rects = TextSelection.rectsBetween(
            page,
            anchor = Offset(250f, 72f),
            focus = Offset(300f, 195f),
        )

        assertTrue("nothing selected", rects.isNotEmpty())
        assertTrue(
            "a drag inside the left column selected ${rects.size} runs; " +
                "the paragraph it covers has about sixteen lines",
            rects.size <= 24,
        )
        rects.forEach { r ->
            assertTrue(
                "rect reaches x=${r.right}, past the column gutter at $GUTTER_LEFT",
                r.right <= GUTTER_LEFT,
            )
        }
    }

    @Test
    fun `dragging down the right column never reaches the left one`() {
        val rects = TextSelection.rectsBetween(
            page,
            anchor = Offset(450f, 72f),
            focus = Offset(450f, 95f),
        )
        assertTrue("nothing selected", rects.isNotEmpty())
        rects.forEach { r ->
            assertTrue(
                "rect starts at x=${r.left}, back inside the left column",
                r.left >= GUTTER_RIGHT,
            )
        }
    }

    @Test
    fun `direction does not change what is selected`() {
        val down = TextSelection.rectsBetween(page, Offset(250f, 72f), Offset(300f, 195f))
        val up = TextSelection.rectsBetween(page, Offset(300f, 195f), Offset(250f, 72f))
        assertEquals(down, up)
    }

    @Test
    fun `lines between the ends are taken whole`() {
        val rects = TextSelection.rectsBetween(page, Offset(300f, 72f), Offset(250f, 130f))
        // The first is trimmed on its left, the last on its right, and every
        // line between them runs the full width of its own run.
        val middle = rects.drop(1).dropLast(1)
        assertTrue("expected lines in between", middle.isNotEmpty())
        middle.forEach { r ->
            val run = page.first { it.top == r.top && it.bottom == r.bottom }
            assertEquals(run.left, r.left, 0.01f)
            assertEquals(run.right, r.right, 0.01f)
        }
    }

    /**
     * The specific defect: clipping a run to a drag that never touched it
     * leaves an *inverted* interval, which is what "no overlap" looks like. The
     * previous code normalised those two edges with min/max, which turned the
     * emptiness into a solid rect spanning the gap instead.
     *
     * Starting the drag past the right-hand end of a run is what exposes it: the
     * clip puts the left edge beyond the right one, and the rect that survives
     * sits entirely off the text.
     */
    @Test
    fun `a run the drag never covered horizontally is dropped, not flipped`() {
        val first = TextSegment(100f, 50f, 200f, 60f, "first line")
        val second = TextSegment(100f, 70f, 200f, 80f, "second line")

        val rects = TextSelection.rectsBetween(
            listOf(first, second),
            anchor = Offset(250f, 55f), // past the end of the first line
            focus = Offset(150f, 75f), // halfway along the second
        )

        assertEquals("the first line was not covered by the drag", 1, rects.size)
        assertEquals(100f, rects[0].left, 0.01f)
        assertEquals(150f, rects[0].right, 0.01f)
    }

    /**
     * The invariant behind the bug, checked over the whole real page: a
     * highlight only ever covers text. A rect that is not inside the run it came
     * from is paint on blank paper.
     */
    @Test
    fun `every rect stays within the run it came from`() {
        val drags = listOf(
            Offset(250f, 72f) to Offset(300f, 195f),
            Offset(374f, 72f) to Offset(200f, 250f),
            Offset(450f, 72f) to Offset(500f, 95f),
            Offset(600f, 300f) to Offset(250f, 600f),
        )
        drags.forEach { (anchor, focus) ->
            TextSelection.rectsBetween(page, anchor, focus).forEach { r ->
                val enclosing = page.any {
                    it.top == r.top && it.bottom == r.bottom &&
                        r.left >= it.left - TOLERANCE && r.right <= it.right + TOLERANCE
                }
                assertTrue("$r lies outside every run on the page", enclosing)
            }
        }
    }

    @Test
    fun `an empty page selects nothing`() {
        assertTrue(
            TextSelection.rectsBetween(emptyList(), Offset(1f, 1f), Offset(2f, 2f)).isEmpty(),
        )
    }

    private companion object {
        const val FIXTURE = "/catalogue-page-145-runs.tsv"

        /** Points. Absorbs the two-decimal rounding in the fixture. */
        const val TOLERANCE = 0.01f

        /** Right edge of the left column, and left edge of the right one. */
        const val GUTTER_LEFT = 380f
        const val GUTTER_RIGHT = 380f
    }
}
