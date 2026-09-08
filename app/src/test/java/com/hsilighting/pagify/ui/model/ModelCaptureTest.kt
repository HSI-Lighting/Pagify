package com.hsilighting.pagify.ui.model

import com.hsilighting.pagify.core.CaptureFormat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

/**
 * How large a picture of the model to draw.
 *
 * The rendering itself belongs to the engine and is covered in Rust. What is
 * here is the arithmetic between a view and a bitmap, which fails quietly:
 * the wrong proportions do not throw, they just reframe the part, and a
 * multiple of a large screen does not throw either — it exhausts memory on a
 * device nobody tested on.
 */
class ModelCaptureTest {

    /** The whole point of a capture: sharper than what is on screen. */
    @Test
    fun a_picture_is_larger_than_the_view() {
        val size = captureSize(viewWidth = 800, viewHeight = 1000, scale = 2)

        assertEquals(1600, size.width)
        assertEquals(2000, size.height)
    }

    /**
     * And the ceiling does not quietly undo that on an ordinary phone.
     *
     * Twice a 1080 × 1920 screen is 8.3 megapixels, so a ceiling set for
     * safety alone would cap the common case and leave every capture barely
     * larger than a screenshot — the feature would still work, and still be
     * pointless, which is the kind of regression nothing else would catch.
     */
    @Test
    fun a_phone_at_twice_size_is_not_cut_down() {
        val size = captureSize(viewWidth = 1080, viewHeight = 1920, scale = 2)

        assertEquals(2160, size.width)
        assertEquals(3840, size.height)
    }

    /**
     * And framed identically.
     *
     * The camera fills whatever it is given, so different proportions mean a
     * different picture from the one the reader was looking at when they
     * pressed the button.
     */
    @Test
    fun the_view_s_proportions_are_kept() {
        for (scale in 1..4) {
            for ((wide, high) in listOf(1080 to 1920, 1440 to 2960, 2408 to 1080, 800 to 800)) {
                val size = captureSize(wide, high, scale)
                val was = wide.toDouble() / high
                val now = size.width.toDouble() / size.height

                assertTrue(
                    "$wide x $high at ${scale}x became ${size.width} x ${size.height}",
                    abs(was - now) < 0.01,
                )
            }
        }
    }

    /**
     * A large screen at a large multiple is held to the ceiling.
     *
     * Four times a tablet is thirty-four megapixels — a 138 MB bitmap, and the
     * app is killed rather than the capture failing, which gives nobody a clue
     * what happened.
     */
    @Test
    fun an_enormous_capture_is_held_to_the_ceiling() {
        val size = captureSize(viewWidth = 2560, viewHeight = 1600, scale = 4)
        val pixels = size.width.toLong() * size.height.toLong()

        assertTrue("$pixels pixels is over the ceiling", pixels <= MOST_CAPTURE_PIXELS)
        // Still worth taking: it did not collapse to something tiny.
        assertTrue("shrank to ${size.width} wide", size.width >= 2560)
    }

    /**
     * But never below the view.
     *
     * A picture coarser than the screen it was taken from is worse than not
     * offering one, and the ceiling is the thing that yields.
     */
    @Test
    fun a_capture_is_never_coarser_than_the_screen() {
        val size = captureSize(viewWidth = 3840, viewHeight = 2160, scale = 4, mostPixels = 100_000)

        assertTrue("${size.width} x ${size.height}", size.width >= 3840 && size.height >= 2160)
    }

    /** A view with no size yet asks for nothing. */
    @Test
    fun nothing_is_drawn_before_the_view_has_a_size() {
        assertEquals(0, captureSize(0, 0, 2).width)
        assertEquals(0, captureSize(1080, 0, 2).height)
    }

    // ---- what the file is called ---------------------------------------------

    /**
     * The part's name comes first.
     *
     * A folder of captures sorts by name, and a timestamp alone says nothing
     * about which part is in the picture.
     */
    @Test
    fun a_capture_is_named_after_the_model() {
        val name = captureFileName("IMPELLER.stp", "2026-09-08 14-31-02", CaptureFormat.PNG)

        assertEquals("IMPELLER 2026-09-08 14-31-02.png", name)
    }

    @Test
    fun the_format_decides_the_extension() {
        val name = captureFileName("valve.STEP", "2026-09-08 14-31-02", CaptureFormat.JPEG)

        assertTrue(name, name.endsWith(".jpg"))
    }

    /** A model with no name still produces a usable file name. */
    @Test
    fun an_unnamed_model_still_gets_a_file_name() {
        val name = captureFileName("", "2026-09-08 14-31-02", CaptureFormat.PNG)

        assertEquals("Model 2026-09-08 14-31-02.png", name)
    }
}
