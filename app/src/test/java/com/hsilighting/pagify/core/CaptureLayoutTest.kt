package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
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
    fun `a zoomed page sits at its offset, scaled`() {
        // The zoomed view draws the page translated and scaled about its own
        // top-left, so this is the rectangle a capture there crops against.
        assertEquals(
            Rect(0f, 0f, 1000f, 2000f),
            zoomedPageBounds(Offset.Zero, baseWidthPx = 500f, baseHeightPx = 1000f, scale = 2f),
        )
    }

    @Test
    fun `a page zoomed past the viewport keeps its negative offset`() {
        // The normal case once zoomed in: the page is bigger than the screen and
        // its top-left is off it. Clamping that to zero would crop the wrong part.
        val bounds = zoomedPageBounds(
            Offset(-300f, -700f),
            baseWidthPx = 500f,
            baseHeightPx = 1000f,
            scale = 4f,
        )
        assertEquals(Rect(-300f, -700f, 1700f, 3300f), bounds)
    }

    @Test
    fun `a crop taken from a zoomed page is in that page's own points`() {
        // End to end for the zoomed path: a box over the middle of a page
        // magnified 4x should crop the middle of the page, not four times it.
        val page = PageSize(widthPoints = 500f, heightPoints = 1000f)
        val bounds = zoomedPageBounds(Offset(-500f, -1000f), 500f, 1000f, scale = 4f)

        val tiles = captureTilesFor(Rect(0f, 0f, 400f, 400f), listOf(PlacedPage(0, bounds, page)))

        assertEquals(1, tiles.size)
        // The box starts 500 px into a 2000 px-wide render of a 500 pt page, so
        // 125 pt in, and covers 400 px = 100 pt.
        assertEquals(Rect(125f, 250f, 225f, 350f), tiles[0].crop)
    }

    @Test
    fun `tiles come back in page order`() {
        // The engine draws them in order, so a later tile paints over an earlier
        // one. Page order is the only order that matches what is on screen.
        val tiles = captureTilesFor(Rect(0f, 0f, 500f, 2020f), spread())
        assertEquals(listOf(0, 1), tiles.map { it.pageIndex })
    }

    // -------------------------------------------------------------- the lasso --

    /** A ring drawn around something in the middle of the first page. */
    private fun ring(): List<Offset> = listOf(
        Offset(100f, 200f),
        Offset(300f, 180f),
        Offset(320f, 400f),
        Offset(120f, 420f),
    )

    @Test
    fun `a ring is captured as its bounding box`() {
        val box = lassoBounds(ring())
        assertEquals(Rect(100f, 180f, 320f, 420f), box)
    }

    @Test
    fun `a ring too small to mean it is not a capture`() {
        val smudge = listOf(Offset(100f, 100f), Offset(104f, 101f), Offset(102f, 104f))
        assertEquals(null, lassoBounds(smudge))
    }

    @Test
    fun `two points enclose nothing`() {
        assertEquals(null, lassoBounds(listOf(Offset(0f, 0f), Offset(500f, 500f))))
        assertEquals(emptyList<Offset>(), captureMaskFor(Rect(0f, 0f, 500f, 500f), listOf(Offset(0f, 0f))))
    }

    @Test
    fun `the mask is in the picture's coordinates, not the reader's`() {
        val outline = ring()
        val box = lassoBounds(outline)!!
        val mask = captureMaskFor(box, outline)

        // The ring's own top-left corner is the picture's origin, so the leftmost
        // point sits at x = 0 and the topmost at y = 0.
        assertEquals(0f, mask.minOf { it.x }, 0.001f)
        assertEquals(0f, mask.minOf { it.y }, 0.001f)
        // And the shape is unchanged — a translation, not a rescale.
        assertEquals(box.width, mask.maxOf { it.x }, 0.001f)
        assertEquals(box.height, mask.maxOf { it.y }, 0.001f)
    }

    @Test
    fun `the mask lines up with the tiles it will be drawn over`() {
        // Both are relative to the same origin. This is the pairing that matters:
        // a mask in one frame and tiles in another produces a picture of the right
        // region with the wrong part of it erased.
        val outline = ring()
        val box = lassoBounds(outline)!!
        val tile = captureTilesFor(box, spread()).single()
        val mask = captureMaskFor(box, outline)

        assertEquals(0f, tile.dest.left, 0.001f)
        assertEquals(0f, tile.dest.top, 0.001f)
        assertTrue(mask.all { it.x >= -0.001f && it.x <= tile.dest.width + 0.001f })
    }

    @Test
    fun `a ring drawn across a page join still gives one picture`() {
        val across = listOf(
            Offset(100f, 900f),
            Offset(400f, 900f),
            Offset(400f, 1200f),
            Offset(100f, 1200f),
        )
        val box = lassoBounds(across)!!
        assertEquals(listOf(0, 1), captureTilesFor(box, spread()).map { it.pageIndex })
        assertEquals(4, captureMaskFor(box, across).size)
    }

    @Test
    fun `an unmeasured point cannot produce a capture`() {
        val broken = listOf(Offset(0f, 0f), Offset(Float.NaN, 10f), Offset(10f, 10f))
        assertEquals(null, lassoBounds(broken))
    }

    @Test
    fun `a straight drag in lasso mode encloses nothing`() {
        // Observed on a phone: this passed every size check — the box is 500 by
        // 500 — and came back as a blank grey picture, because a line encloses no
        // area for the mask to keep.
        val straight = (0..20).map { Offset(200f + it * 25f, 400f + it * 25f) }
        assertEquals(null, lassoBounds(straight))
    }

    @Test
    fun `a ring around one line of text is thin but real`() {
        // 500 by 9. The fullness floor is about rings that enclose nothing,
        // not about rings that are narrow.
        val smear = listOf(
            Offset(100f, 400f),
            Offset(600f, 403f),
            Offset(600f, 409f),
            Offset(100f, 406f),
        )
        // Thin, but not blank: this is what ringing a single line of text
        // gives, and the strip it produces is exactly what was drawn.
        assertEquals(Rect(100f, 400f, 600f, 409f), lassoBounds(smear))
    }

    @Test
    fun `a crescent that fills little of its box is still a ring`() {
        // The shape the fullness floor must not reject: a thin arc around the edge
        // of a drawing fills a small part of its bounding box and is exactly what
        // someone means to draw.
        val crescent = listOf(
            Offset(100f, 100f),
            Offset(300f, 130f),
            Offset(500f, 100f),
            Offset(500f, 150f),
            Offset(300f, 190f),
            Offset(100f, 150f),
        )
        assertEquals(Rect(100f, 100f, 500f, 190f), lassoBounds(crescent))
    }

    @Test
    fun `a ring drawn the other way round is the same ring`() {
        // Shoelace area is signed; winding must not decide whether a capture
        // happens.
        val clockwise = ring()
        val anticlockwise = ring().reversed()
        assertEquals(lassoBounds(clockwise), lassoBounds(anticlockwise))
    }
}
