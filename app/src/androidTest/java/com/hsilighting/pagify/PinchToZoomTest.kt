package com.hsilighting.pagify

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.ui.components.pinchToZoom
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Contract tests for the pinch handler.
 *
 * These exist because `adb` has no multitouch primitive and SELinux blocks writing
 * synthetic events to `/dev/input`, so a two-finger gesture cannot be driven from
 * the shell. Compose's test framework can inject multiple pointers directly, which
 * makes this the only way to actually exercise the handler rather than assume it.
 *
 * Every test drives *both* pointers before a single `move()`, because that is what
 * a real two-finger gesture delivers: one event carrying both positions. Updating
 * one pointer per event describes a gesture nobody performs.
 */
@RunWith(AndroidJUnit4::class)
class PinchToZoomTest {

    @get:Rule
    val rule = createComposeRule()

    private val reported = mutableListOf<Float>()
    private var gesturesEnded = 0

    private fun setUpSurface() {
        rule.setContent {
            Box(
                Modifier
                    .fillMaxSize()
                    .testTag(TAG)
                    .pinchToZoom(onGestureEnd = { gesturesEnded++ }) { factor, _ ->
                        reported += factor
                    },
            )
        }
    }

    @Test
    fun twoFingersMovingApartReportZoomIn() {
        setUpSurface()

        rule.onNodeWithTag(TAG).performTouchInput {
            val y = height / 2f
            val cx = width / 2f
            down(0, Offset(cx - 100f, y))
            down(1, Offset(cx + 100f, y))

            repeat(4) { step ->
                val spread = 100f + (step + 1) * 80f
                updatePointerTo(0, Offset(cx - spread, y))
                updatePointerTo(1, Offset(cx + spread, y))
                move()
            }
            up(0)
            up(1)
        }

        assertTrue("the handler reported nothing at all", reported.isNotEmpty())
        assertTrue(
            "spreading fingers must report factors above 1, got $reported",
            reported.all { it > 1f },
        )
        assertEquals("the gesture end must be reported once", 1, gesturesEnded)
    }

    @Test
    fun twoFingersComingTogetherReportZoomOut() {
        setUpSurface()

        rule.onNodeWithTag(TAG).performTouchInput {
            val y = height / 2f
            val cx = width / 2f
            down(0, Offset(cx - 400f, y))
            down(1, Offset(cx + 400f, y))

            repeat(4) { step ->
                val spread = 400f - (step + 1) * 80f
                updatePointerTo(0, Offset(cx - spread, y))
                updatePointerTo(1, Offset(cx + spread, y))
                move()
            }
            up(0)
            up(1)
        }

        assertTrue("the handler reported nothing at all", reported.isNotEmpty())
        assertTrue(
            "pinching inwards must report factors below 1, got $reported",
            reported.all { it < 1f },
        )
    }

    /**
     * The reported bug: two fingers dragging to scroll would quietly zoom in.
     *
     * The separation wobbles by a few pixels because fingers are not a rigid bar,
     * and each wobble arrived as a zoom factor slightly off 1.0. Acting on them
     * accumulated into a real magnification, and the reader jumped into the
     * magnified view on its own.
     */
    @Test
    fun aTwoFingerScrollNeverReportsZoom() {
        setUpSurface()

        rule.onNodeWithTag(TAG).performTouchInput {
            val cx = width / 2f
            val startY = height * 0.8f
            down(0, Offset(cx - 200f, startY))
            down(1, Offset(cx + 200f, startY))

            repeat(15) { step ->
                // A few pixels of separation noise, the way real fingers behave,
                // against a scroll of several hundred.
                val wobble = ((step % 4) - 2) * 3f
                val y = startY - (step + 1) * 18f
                updatePointerTo(0, Offset(cx - 200f - wobble, y))
                updatePointerTo(1, Offset(cx + 200f + wobble, y))
                move()
            }
            up(0)
            up(1)
        }

        assertTrue(
            "a parallel two-finger drag must not zoom, got $reported",
            reported.isEmpty(),
        )
    }

    /**
     * The decision has to hold for the whole gesture. Fingers routinely drift
     * apart at the end of a long scroll, and if that were allowed to re-classify,
     * the page would start zooming under a finger already on its way up.
     */
    @Test
    fun aScrollThatDriftsApartAtTheEndStaysAScroll() {
        setUpSurface()

        rule.onNodeWithTag(TAG).performTouchInput {
            val cx = width / 2f
            val startY = height * 0.8f
            down(0, Offset(cx - 150f, startY))
            down(1, Offset(cx + 150f, startY))

            // First scroll far enough to be unambiguously a pan...
            repeat(10) { step ->
                val y = startY - (step + 1) * 18f
                updatePointerTo(0, Offset(cx - 150f, y))
                updatePointerTo(1, Offset(cx + 150f, y))
                move()
            }
            // ...then let the fingers splay well past the slop.
            repeat(6) { step ->
                val spread = 150f + (step + 1) * 40f
                val y = startY - 180f
                updatePointerTo(0, Offset(cx - spread, y))
                updatePointerTo(1, Offset(cx + spread, y))
                move()
            }
            up(0)
            up(1)
        }

        assertTrue(
            "a gesture already committed to scrolling must not become a zoom, got $reported",
            reported.isEmpty(),
        )
    }

    /**
     * The other side of the same coin: a pinch with one finger held still moves
     * the midpoint about half as far as it changes the separation, so it must not
     * be mistaken for a pan.
     */
    @Test
    fun aPinchAnchoredOnOneFingerStillZooms() {
        setUpSurface()

        rule.onNodeWithTag(TAG).performTouchInput {
            val y = height / 2f
            val cx = width / 2f
            val anchor = Offset(cx - 100f, y)
            down(0, anchor)
            down(1, Offset(cx + 100f, y))

            repeat(5) { step ->
                updatePointerTo(0, anchor)
                updatePointerTo(1, Offset(cx + 100f + (step + 1) * 90f, y))
                move()
            }
            up(0)
            up(1)
        }

        assertTrue(
            "an anchored pinch must still zoom, got $reported",
            reported.isNotEmpty() && reported.all { it > 1f },
        )
    }

    @Test
    fun aSingleFingerDragIsIgnoredSoScrollingStillWorks() {
        setUpSurface()

        rule.onNodeWithTag(TAG).performTouchInput {
            swipe(
                start = Offset(width / 2f, height * 0.8f),
                end = Offset(width / 2f, height * 0.2f),
                durationMillis = 200,
            )
        }

        assertTrue(
            "one finger must never be treated as zoom, got $reported",
            reported.isEmpty(),
        )
        assertEquals("one finger is not a two-finger gesture", 0, gesturesEnded)
    }

    private companion object {
        const val TAG = "zoom-surface"
    }
}
