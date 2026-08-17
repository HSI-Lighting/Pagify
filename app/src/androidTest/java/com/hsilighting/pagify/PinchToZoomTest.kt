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
 * The second test matters as much as the first: the handler must ignore
 * single-pointer gestures, or it starves the surrounding scroll containers and the
 * document becomes unscrollable.
 */
@RunWith(AndroidJUnit4::class)
class PinchToZoomTest {

    @get:Rule
    val rule = createComposeRule()

    private val reported = mutableListOf<Float>()

    private fun setUpSurface() {
        rule.setContent {
            Box(
                Modifier
                    .fillMaxSize()
                    .testTag(TAG)
                    .pinchToZoom { factor -> reported += factor },
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

            // Both pointers updated before a single `move`, so the handler sees one
            // event containing both — which is what a real pinch delivers.
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
    }

    private companion object {
        const val TAG = "zoom-surface"
    }
}
