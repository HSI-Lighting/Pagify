package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The settings file.
 *
 * Lenient on the way in, because the file is a convenience and the defaults are
 * always a usable answer — but lenient in the *right* way: one unreadable key
 * must not cost the others.
 */
class AppSettingsTest {

    @Test
    fun `settings round trip`() {
        val settings = AppSettings(
            theme = ThemeChoice.DARK,
            showViewfinder = false,
            viewfinderMinimized = true,
        )
        assertEquals(settings, settingsFromJson(settings.toSettingsJson()))
    }

    @Test
    fun `defaults are what a fresh install shows`() {
        val fresh = AppSettings()

        assertEquals(ThemeChoice.SYSTEM, fresh.theme)
        assertTrue("the viewfinder should be on to begin with", fresh.showViewfinder)
        assertTrue("and open rather than folded", !fresh.viewfinderMinimized)
    }

    @Test
    fun `a file from an older build keeps what it does have`() {
        // Written before the viewfinder settings existed. Losing the theme over a
        // key that had not been invented yet would be a poor trade.
        val settings = settingsFromJson("""{"theme":"LIGHT"}""")

        assertEquals(ThemeChoice.LIGHT, settings.theme)
        assertEquals(AppSettings().showViewfinder, settings.showViewfinder)
    }

    @Test
    fun `nonsense on disk reads as the defaults`() {
        assertEquals(AppSettings(), settingsFromJson("not json at all"))
        assertEquals(AppSettings(), settingsFromJson(""))
    }

    @Test
    fun `an unreadable theme does not take the viewfinder with it`() {
        val settings = settingsFromJson("""{"theme":"MIDNIGHT","showViewfinder":false}""")

        assertEquals(ThemeChoice.SYSTEM, settings.theme)
        assertEquals(false, settings.showViewfinder)
    }

    @Test
    fun `the handle position round trips`() {
        val settings = AppSettings(viewfinderHandleX = 0.25f, viewfinderHandleY = 0.75f)
        val read = settingsFromJson(settings.toSettingsJson())

        assertEquals(0.25f, read.viewfinderHandleX, 0.0001f)
        assertEquals(0.75f, read.viewfinderHandleY, 0.0001f)
    }

    @Test
    fun `a handle position off the screen is pulled back on to it`() {
        // Fractions, so anything outside 0..1 is a handle that cannot be reached.
        // A file edited by hand, or written by a build with a different idea of
        // the coordinates, should not cost the user their way back to the map.
        val read = settingsFromJson(
            """{"viewfinderHandleX":4.5,"viewfinderHandleY":-2.0}""",
        )

        assertEquals(1f, read.viewfinderHandleX, 0.0001f)
        assertEquals(0f, read.viewfinderHandleY, 0.0001f)
    }

    @Test
    fun `how a picture was last exported survives a round trip`() {
        val chosen = AppSettings(
            captureScale = CaptureScale.LOW,
            captureFormat = CaptureFormat.JPEG,
            captureFill = CaptureFill.TRANSPARENT,
        )

        val read = settingsFromJson(chosen.toSettingsJson())

        assertEquals(CaptureScale.LOW, read.captureScale)
        assertEquals(CaptureFormat.JPEG, read.captureFormat)
        assertEquals(CaptureFill.TRANSPARENT, read.captureFill)
    }

    @Test
    fun `a fresh install exports at the best quality it can`() {
        // A picture that comes out too coarse cannot be sharpened afterwards, so
        // the default is the one that cannot be regretted.
        assertEquals(CaptureScale.HIGH, AppSettings().captureScale)
        assertEquals(CaptureFormat.PNG, AppSettings().captureFormat)
    }

    @Test
    fun `an export setting written by another build falls back rather than shifting`() {
        // Stored by name, not by ordinal. The scales were renumbered when they
        // stopped being 1x, 2x and 4x, and an ordinal written before that would
        // have come back meaning a different sharpness — silently.
        val read = settingsFromJson("""{"captureScale":"X2","captureFormat":"WEBP"}""")

        assertEquals(CaptureScale.HIGH, read.captureScale)
        assertEquals(CaptureFormat.PNG, read.captureFormat)
    }

    @Test
    fun `the handle starts where the viewfinder itself appears`() {
        // Folding it away should not also move it: the map sits at the right-hand
        // edge, halfway down, and so does the handle it leaves behind.
        val fresh = AppSettings()

        assertEquals(1f, fresh.viewfinderHandleX, 0.0001f)
        assertEquals(0.5f, fresh.viewfinderHandleY, 0.0001f)
    }
}
