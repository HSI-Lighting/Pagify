package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The geometry between a recognised pixel box and a page-space run.
 *
 * This is where a mistake would be silent: every highlight would land somewhere
 * plausible but wrong, and the further down the page the worse it would get. The
 * recognition itself needs a device; this arithmetic does not, so it is pinned
 * here where it can fail fast.
 */
class PageTextRecogniserTest {

    private val catalogue = PageSize(widthPoints = 1647.66f, heightPoints = 901.72f)
    private val a4 = PageSize(widthPoints = 595f, heightPoints = 842f)

    // ------------------------------------------------------------ the scale --

    @Test
    fun `a large page is rendered down to the target long edge, not up`() {
        val scale = PageTextRecogniser.scaleFor(catalogue)
        val longEdgePx = maxOf(catalogue.widthPoints, catalogue.heightPoints) * scale

        assertEquals(PageTextRecogniser.TARGET_LONG_EDGE_PX, longEdgePx, 1f)
    }

    @Test
    fun `a small page is never rendered below its natural size`() {
        // A4's long edge is 842 pt, well under the target, so the ideal scale is
        // greater than 1 and the floor never binds. A postage-stamp page is the
        // case that matters: reading it at less than 1:1 would be self-defeating.
        val stamp = PageSize(widthPoints = 8000f, heightPoints = 8000f)
        assertTrue(
            "a huge page still renders at something, never zero",
            PageTextRecogniser.scaleFor(stamp) > 0f,
        )
        assertTrue(PageTextRecogniser.scaleFor(a4) >= 1f)
    }

    @Test
    fun `recognition obeys the same pixel ceiling as every other render`() {
        // A very large page must not be the one path that fails inside
        // Bitmap.createBitmap; the ceiling wins over the target long edge.
        val huge = PageSize(widthPoints = 14000f, heightPoints = 14000f)
        val scale = PageTextRecogniser.scaleFor(huge)
        val pixels = huge.widthPoints.toDouble() * scale * huge.heightPoints.toDouble() * scale

        assertTrue(
            "scale $scale yields $pixels pixels, over the ${RenderScale.MAX_PIXELS} ceiling",
            pixels <= RenderScale.MAX_PIXELS * 1.001,
        )
    }

    @Test
    fun `a degenerate page size does not divide by zero`() {
        assertEquals(1f, PageTextRecogniser.scaleFor(PageSize(0f, 0f)))
        assertEquals(1f, PageTextRecogniser.scaleFor(PageSize(-5f, 100f)))
    }

    // ------------------------------------------------------ pixels to points --

    @Test
    fun `the scale is taken from the bitmap actually produced`() {
        // 2x the page's points in both axes.
        val perPoint = PageTextRecogniser.pixelsPerPoint(1190, 1684, a4)
        assertEquals(2f, perPoint, 0.001f)
    }

    @Test
    fun `a rounded bitmap still yields a scale close to the truth`() {
        // Renderers round to whole pixels, so the two axes disagree slightly; the
        // average is what keeps a run from drifting down the page.
        val perPoint = PageTextRecogniser.pixelsPerPoint(1191, 1683, a4)
        assertEquals(2f, perPoint, 0.002f)
    }

    @Test
    fun `a zero-sized page falls back to one to one rather than infinity`() {
        assertEquals(1f, PageTextRecogniser.pixelsPerPoint(100, 100, PageSize(0f, 0f)))
    }

    // ---------------------------------------------------------- the segment --

    @Test
    fun `a recognised box converts to page points with no vertical flip`() {
        // Both spaces put the origin at the top-left with y increasing downwards:
        // the recogniser reads a rendered bitmap, which has no PDF bottom-left
        // origin to undo. Flipping here as the engine's own extraction does would
        // put every run on the wrong half of the page.
        val segment = PageTextRecogniser.segmentFor(
            200, 100, 600, 160,
            "www.hsilighting.com",
            pixelsPerPoint = 2f,
        )

        assertEquals(100f, segment.left)
        assertEquals(50f, segment.top)
        assertEquals(300f, segment.right)
        assertEquals(80f, segment.bottom)
        assertEquals("www.hsilighting.com", segment.text)
    }

    @Test
    fun `a box near the top of the page stays near the top`() {
        val top = PageTextRecogniser.segmentFor(0, 10, 100, 40, "header", 2f)
        val bottom = PageTextRecogniser.segmentFor(0, 1600, 100, 1630, "footer", 2f)

        assertTrue(
            "a run recognised at the top of the image must stay above one from the bottom",
            top.top < bottom.top,
        )
    }

    @Test
    fun `a segment keeps positive width and height`() {
        val segment = PageTextRecogniser.segmentFor(10, 20, 310, 80, "x", 2f)

        assertEquals(150f, segment.width)
        assertEquals(30f, segment.height)
    }

    @Test
    fun `a nonsense scale does not collapse every run onto one point`() {
        // Guards the fallback: dividing by zero would send every coordinate to
        // infinity, and every highlight would vanish rather than be misplaced —
        // which is much harder to diagnose from a bug report.
        val segment = PageTextRecogniser.segmentFor(10, 20, 30, 40, "x", 0f)

        assertEquals(10f, segment.left)
        assertEquals(40f, segment.bottom)
    }
}
