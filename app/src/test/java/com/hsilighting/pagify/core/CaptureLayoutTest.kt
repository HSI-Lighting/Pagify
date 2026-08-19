package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Turning a dragged box into per-page tiles.
 *
 * The reason this is a pure function with tests of its own: it is wrong in ways
 * that look right. A tile off by the height of a page gap, or a crop derived from
 * the wrong page's scale, produces a capture that is plausible on screen and
 * simply shows the wrong part of the document.
 */
class CaptureLayoutTest {

    /** Two A4 pages drawn 500 px wide, stacked with a 20 px gap. */
    private fun spread(): List<PlacedPage> {
        val size = PageSize(widthPoints = 500f, heightPoints = 1000f)
        return listOf(
            PlacedPage(0, Rect(0f, 0f, 500f, 1000f), size),
            PlacedPage(1, Rect(0f, 1020f, 500f, 2020f), size),
        )
    }

    @Test
    fun `a box inside one page gives one tile`() {
        val tiles = captureTilesFor(Rect(100f, 100f, 300f, 400f), spread())

        assertEquals(1, tiles.size)
        assertEquals(0, tiles[0].pageIndex)
        assertEquals(Rect(100f, 100f, 300f, 400f), tiles[0].crop)
        // The picture's origin is the drag's corner, so the tile starts at zero.
        assertEquals(Rect(0f, 0f, 200f, 300f), tiles[0].dest)
    }

    @Test
    fun `a box across the join gives a tile for each page`() {
        // The complaint this feature answers.
        val tiles = captureTilesFor(Rect(0f, 900f, 500f, 1200f), spread())

        assertEquals(2, tiles.size)
        assertEquals(listOf(0, 1), tiles.map { it.pageIndex })

        // The bottom 100 px of page 0, which is its last 100 points.
        assertEquals(Rect(0f, 900f, 500f, 1000f), tiles[0].crop)
        assertEquals(Rect(0f, 0f, 500f, 100f), tiles[0].dest)

        // Then the top 180 px of page 1, placed after the 20 px gap.
        assertEquals(Rect(0f, 0f, 500f, 180f), tiles[1].crop)
        assertEquals(Rect(0f, 120f, 500f, 300f), tiles[1].dest)
    }

    @Test
    fun `the gap between pages belongs to no tile`() {
        // Nothing is rendered there, which is what lets the engine paint it as the
        // reader's own background instead of stretching a page across it.
        val tiles = captureTilesFor(Rect(0f, 900f, 500f, 1200f), spread())

        val upperEnds = tiles[0].dest.bottom
        val lowerStarts = tiles[1].dest.top
        assertEquals(20f, lowerStarts - upperEnds)
    }

    @Test
    fun `pages the box never touches produce no tiles`() {
        val tiles = captureTilesFor(Rect(0f, 0f, 100f, 100f), spread())
        assertEquals(listOf(0), tiles.map { it.pageIndex })
    }

    @Test
    fun `a box reaching past the edge of a page is clipped to it`() {
        // Dragging into the margin either side is normal, and the part outside the
        // page has nothing to render — it becomes background.
        val tiles = captureTilesFor(Rect(-200f, -100f, 700f, 300f), spread())

        assertEquals(1, tiles.size)
        assertEquals(Rect(0f, 0f, 500f, 300f), tiles[0].crop)
        // Placed 200 px in from the drag's left edge, where the page actually is.
        assertEquals(Rect(200f, 100f, 700f, 400f), tiles[0].dest)
    }

    @Test
    fun `each page is measured by its own scale`() {
        // The reader draws every page to the same width, so a wide page is at a
        // very different points-per-pixel from a narrow one. Sharing a scale would
        // crop the wrong part of one of them.
        val pages = listOf(
            PlacedPage(0, Rect(0f, 0f, 500f, 500f), PageSize(1000f, 1000f)),
            PlacedPage(1, Rect(0f, 520f, 500f, 1020f), PageSize(250f, 250f)),
        )

        val tiles = captureTilesFor(Rect(0f, 0f, 250f, 1020f), pages)

        // Half the width of each on screen, but 500 points of one and 125 of the
        // other.
        assertEquals(500f, tiles[0].crop.right)
        assertEquals(125f, tiles[1].crop.right)
    }

    @Test
    fun `a sliver of a page too thin to render is dropped`() {
        val tiles = captureTilesFor(Rect(0f, 999.6f, 500f, 1200f), spread())
        assertTrue(
            "a sub-pixel sliver of page 0 should not have produced a tile",
            tiles.none { it.pageIndex == 0 },
        )
    }

    @Test
    fun `a page with no measured size is skipped rather than dividing by zero`() {
        val pages = listOf(PlacedPage(0, Rect(0f, 0f, 500f, 500f), PageSize(0f, 0f)))
        assertEquals(emptyList<CaptureTile>(), captureTilesFor(Rect(0f, 0f, 100f, 100f), pages))
    }

    @Test
    fun `tiles come back in page order`() {
        // The engine draws them in order, so a later tile paints over an earlier
        // one. Page order is the only order that matches what is on screen.
        val tiles = captureTilesFor(Rect(0f, 0f, 500f, 2020f), spread())
        assertEquals(listOf(0, 1), tiles.map { it.pageIndex })
    }
}
