package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Page points to pixels, through a rotation.
 *
 * The failures worth catching here all look plausible on screen: a mark on the
 * wrong side of the sheet, a mark that mirrors instead of turning, an eraser that
 * finds nothing where the ink clearly is. A round trip alone would not catch the
 * mirror — an inverse derived from the same wrong turn comes back exactly where
 * it started — so the corners are pinned by hand as well.
 */
class PageMappingTest {

    /** An A4-ish page, deliberately not square so a swapped axis shows. */
    private val page = PageSize(400f, 800f)

    private fun mapping(turns: Int, scale: Float = 2f, origin: Offset = Offset.Zero) = PageMapping(
        scale = scale,
        origin = origin,
        quarterTurns = turns,
        pageWidthPoints = page.widthPoints,
        pageHeightPoints = page.heightPoints,
    )

    private fun assertOffset(expected: Offset, actual: Offset) =
        assertEquals("expected $expected but was $actual", 0f, (actual - expected).getDistance(), 0.01f)

    @Test
    fun `upright, a point is scaled and shifted and nothing else`() {
        val at = mapping(turns = 0, origin = Offset(10f, 20f))

        assertOffset(Offset(210f, 220f), at.toScreen(Offset(100f, 100f)))
        assertOffset(Offset(100f, 100f), at.toPage(Offset(210f, 220f)))
    }

    @Test
    fun `a quarter turn sends the top-left corner to the top right`() {
        // Clockwise, matching the renderer. Backwards, this would put it bottom
        // left — still a corner, still a plausible-looking page.
        val at = mapping(turns = 1, scale = 1f)

        assertOffset(Offset(800f, 0f), at.toScreen(Offset(0f, 0f)))
        assertOffset(Offset(800f, 400f), at.toScreen(Offset(400f, 0f)))
        assertOffset(Offset(0f, 0f), at.toScreen(Offset(0f, 800f)))
    }

    @Test
    fun `half a turn puts the top-left corner at the bottom right`() {
        val at = mapping(turns = 2, scale = 1f)

        assertOffset(Offset(400f, 800f), at.toScreen(Offset(0f, 0f)))
        assertOffset(Offset(0f, 0f), at.toScreen(Offset(400f, 800f)))
    }

    @Test
    fun `three quarters sends the top-left corner to the bottom left`() {
        val at = mapping(turns = 3, scale = 1f)

        assertOffset(Offset(0f, 400f), at.toScreen(Offset(0f, 0f)))
        assertOffset(Offset(800f, 400f), at.toScreen(Offset(0f, 800f)))
    }

    @Test
    fun `a turned page is drawn across its own height`() {
        // What the layout depends on: at a quarter turn the drawn width comes from
        // the page's height, so a scale derived from the turned width fills the box.
        val at = mapping(turns = 1, scale = 1f)
        val corners = listOf(
            Offset(0f, 0f),
            Offset(400f, 0f),
            Offset(400f, 800f),
            Offset(0f, 800f),
        ).map { at.toScreen(it) }

        assertEquals(800f, corners.maxOf { it.x } - corners.minOf { it.x }, 0.01f)
        assertEquals(400f, corners.maxOf { it.y } - corners.minOf { it.y }, 0.01f)
    }

    @Test
    fun `a touch comes back as the point it was drawn from, at every turn`() {
        val point = Offset(137f, 611f)

        (0..3).forEach { turns ->
            val at = mapping(turns, scale = 1.7f, origin = Offset(31f, 47f))
            assertOffset(point, at.toPage(at.toScreen(point)))
        }
    }

    @Test
    fun `negative and oversized turns mean the same as their quarter`() {
        // The reader counts turns up without wrapping in one place and down in
        // another; both have to land on the same page.
        val point = Offset(100f, 700f)

        assertOffset(mapping(1).toScreen(point), mapping(5).toScreen(point))
        assertOffset(mapping(3).toScreen(point), mapping(-1).toScreen(point))
    }

    @Test
    fun `a rectangle keeps a positive width once it has been turned`() {
        // A quarter turn sends a top-left corner to a top-right one. Left as-is,
        // the rectangle would have negative width and draw as nothing.
        val at = mapping(turns = 1, scale = 1f)
        val box = at.toScreen(Rect(50f, 100f, 150f, 140f))

        assertTrue("inside out: $box", box.width > 0f && box.height > 0f)
        // The long side was horizontal on the page, so it is vertical now.
        assertEquals(40f, box.width, 0.01f)
        assertEquals(100f, box.height, 0.01f)
    }

    @Test
    fun `lengths scale but do not turn`() {
        val at = mapping(turns = 1, scale = 3f)

        assertEquals(6f, at.toScreen(2f), 0.001f)
        assertEquals(2f, at.toPage(6f), 0.001f)
    }

    @Test
    fun `a quarter turn swaps the page's width and height, and a half turn does not`() {
        // What the layout is built on: the box a turned page is drawn in, and the
        // render scale that fills it, both come from this.
        assertEquals(PageSize(841.89f, 595.28f), PageSize(595.28f, 841.89f).turned(1))
        assertEquals(PageSize(595.28f, 841.89f), PageSize(595.28f, 841.89f).turned(2))
        assertEquals(PageSize(841.89f, 595.28f), PageSize(595.28f, 841.89f).turned(3))
        assertEquals(PageSize(595.28f, 841.89f), PageSize(595.28f, 841.89f).turned(0))
    }

    @Test
    fun `the sheet's bounds are the page and nothing around it`() {
        // What ink is clipped to. The page is drawn into a taller row with grey
        // either side, and this is the part of it that is paper.
        val at = mapping(turns = 0, scale = 2f, origin = Offset(10f, 20f))
        val sheet = at.screenBounds

        assertEquals(10f, sheet.left, 0.01f)
        assertEquals(20f, sheet.top, 0.01f)
        assertEquals(10f + 800f, sheet.right, 0.01f)
        assertEquals(20f + 1600f, sheet.bottom, 0.01f)
    }

    @Test
    fun `a turned page's bounds turn with it`() {
        // And stay a rectangle with positive width: a quarter turn sends the
        // top-left corner to the top right, and corners left in their original
        // roles would give an inside-out rectangle that clips everything away.
        val sheet = mapping(turns = 1, scale = 1f).screenBounds

        assertEquals(800f, sheet.width, 0.01f)
        assertEquals(400f, sheet.height, 0.01f)
    }

    @Test
    fun `a point off the page is held at the edge`() {
        // Held rather than dropped: a stroke taken past the margin should run
        // along it. Dropping the points would break the stroke into pieces.
        val at = mapping(turns = 0)

        assertOffset(Offset(0f, 0f), at.clampToPage(Offset(-50f, -50f)))
        assertOffset(Offset(400f, 800f), at.clampToPage(Offset(999f, 999f)))
        assertOffset(Offset(400f, 400f), at.clampToPage(Offset(500f, 400f)))
    }

    @Test
    fun `a point on the page is left alone`() {
        val at = mapping(turns = 0)
        assertOffset(Offset(137f, 611f), at.clampToPage(Offset(137f, 611f)))
    }

    @Test
    fun `an unmeasured page clamps nothing rather than clamping to a dot`() {
        // Zero-by-zero bounds would pin every point of a stroke to the corner,
        // which is a worse failure than letting it through until the page is known.
        assertOffset(
            Offset(137f, 611f),
            PageMapping.Unmeasured.clampToPage(Offset(137f, 611f)),
        )
    }

    @Test
    fun `an unmeasured page converts nothing rather than dividing by zero`() {
        assertFalse(PageMapping.Unmeasured.isUsable)
        assertOffset(Offset.Zero, PageMapping.Unmeasured.toPage(Offset(10f, 10f)))
        assertEquals(0f, PageMapping.Unmeasured.toPage(10f), 0f)
    }
}
