package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The parts of capture that are pure, and therefore testable off-device.
 *
 * The picture itself is the engine's — `rust/pdf_core/tests/region_export.rs`
 * holds the guarantee that a crop contains only what was inside it. What is left
 * here is the arithmetic between a finger and that crop, which is where the
 * mistakes are silent: a rectangle with a negative width still looks like a
 * rectangle.
 */
class CaptureTest {

    @Test
    fun `a rectangle dragged up and to the left is still a rectangle`() {
        // `Rect(topLeft, bottomRight)` does not normalise, so a backwards drag
        // gives negative width and captures nothing at all — with no error to
        // show for it.
        val backwards = rectFromCorners(300f, 400f, 100f, 200f)
        assertEquals(Rect(100f, 200f, 300f, 400f), backwards)
        assertTrue(backwards.width > 0f && backwards.height > 0f)
    }

    @Test
    fun `dragging forwards gives the same rectangle as dragging backwards`() {
        assertEquals(
            rectFromCorners(10f, 20f, 110f, 220f),
            rectFromCorners(110f, 220f, 10f, 20f),
        )
    }

    @Test
    fun `a drag too small to mean it is not worth capturing`() {
        // A tap that moved a couple of points is someone trying to scroll. Acting
        // on it puts a two-pixel image and a share sheet in front of them.
        assertFalse(Rect(10f, 10f, 14f, 14f).isWorthCapturing())
        assertFalse(Rect(10f, 10f, 200f, 12f).isWorthCapturing(), "a sliver is not a region")
    }

    @Test
    fun `a deliberate drag is worth capturing`() {
        assertTrue(Rect(10f, 10f, 210f, 110f).isWorthCapturing())
        // Exactly at the threshold counts: the bound is what is too small, not
        // what is barely enough.
        assertTrue(
            Rect(0f, 0f, MINIMUM_CAPTURE_POINTS, MINIMUM_CAPTURE_POINTS).isWorthCapturing(),
        )
    }

    @Test
    fun `a file name says which document and which page it came from`() {
        assertEquals(
            "Price list p8 2026-08-19 15-04-22.png",
            captureFileName("Price list.pdf", pageIndex = 7, CaptureFormat.PNG, "2026-08-19 15-04-22"),
        )
    }

    @Test
    fun `characters a filesystem would refuse are dropped`() {
        // A PDF's title can hold anything at all, including the separators and
        // reserved characters that make a file unwritable on one platform and
        // fine on another.
        val name = captureFileName(
            "Q3/Q4: <final> \"draft\"|v2*.pdf",
            pageIndex = 0,
            format = CaptureFormat.JPEG,
            timestamp = "2026-01-01 00-00-00",
        )
        assertEquals("Q3Q4 final draftv2 p1 2026-01-01 00-00-00.jpg", name)
    }

    @Test
    fun `a document with no usable name still gets one`() {
        assertEquals(
            "Pagify p1 t.png",
            captureFileName("...", pageIndex = 0, CaptureFormat.PNG, "t"),
        )
    }

    @Test
    fun `a very long title is trimmed rather than making an unwritable name`() {
        val name = captureFileName(
            "x".repeat(300) + ".pdf",
            pageIndex = 0,
            format = CaptureFormat.PNG,
            timestamp = "t",
        )
        assertTrue("name was ${name.length} characters", name.length < 100)
    }

    @Test
    fun `each format carries the extension and mime type that match it`() {
        assertEquals("png", CaptureFormat.PNG.extension)
        assertEquals("image/png", CaptureFormat.PNG.mimeType)
        assertEquals("jpg", CaptureFormat.JPEG.extension)
        assertEquals("image/jpeg", CaptureFormat.JPEG.mimeType)
        // The wire name is what the engine parses; `jpg` is not one of them.
        assertEquals("jpeg", CaptureFormat.JPEG.wireName)
    }

    @Test
    fun `the offered scales are multipliers, sharpest first`() {
        // The order is the order they are offered in, and the first of them is the
        // default: a capture that comes out too coarse cannot be sharpened later,
        // while one that comes out large can always be sent again smaller.
        assertEquals(listOf(4f, 2f, 1f), CaptureScale.entries.map { it.factor })
        assertEquals(listOf("Hi", "Mid", "Lo"), CaptureScale.entries.map { it.label })
    }

    private fun assertFalse(condition: Boolean, message: String) =
        assertFalse(message, condition)
}
