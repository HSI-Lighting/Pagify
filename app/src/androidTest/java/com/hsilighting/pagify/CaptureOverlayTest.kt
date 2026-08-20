package com.hsilighting.pagify

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.ui.components.captureOverlay
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Contract tests for the capture gesture, box and ring.
 *
 * Here rather than in the JVM tier because the geometry is already tested there
 * and this is about the *gesture*: which drag becomes a capture, and what the
 * handler reports when it does. And here rather than driven from `adb`, because
 * the shell cannot draw a curve — `input swipe` interpolates a straight line, and
 * separate `input motionevent` calls do not arrive as one gesture. A ring is the
 * one shape the shell cannot express, which makes it the one shape most worth
 * pinning down.
 */
@RunWith(AndroidJUnit4::class)
class CaptureOverlayTest {

    @get:Rule
    val rule = createComposeRule()

    private val captured = mutableListOf<Pair<Rect, List<Offset>>>()

    private fun setUpSurface(lasso: Boolean) {
        captured.clear()
        rule.setContent {
            Box(
                Modifier
                    .fillMaxSize()
                    .testTag(TAG)
                    .captureOverlay(lasso = lasso) { rect, ring -> captured += rect to ring },
            )
        }
    }

    @Test
    fun aRingReportsItsBoundingBoxAndTheRingItself() {
        setUpSurface(lasso = true)

        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(200f, 200f))
            moveTo(Offset(600f, 200f))
            moveTo(Offset(600f, 600f))
            moveTo(Offset(200f, 600f))
            moveTo(Offset(200f, 220f))
            up()
        }

        assertEquals(1, captured.size)
        val (box, ring) = captured.single()
        assertEquals(200f, box.left, SLOP)
        assertEquals(200f, box.top, SLOP)
        assertEquals(600f, box.right, SLOP)
        assertEquals(600f, box.bottom, SLOP)

        // The ring is a shape, not the two ends of one. Each injected `moveTo` is a
        // single event — the framework does not interpolate the way a finger does —
        // so this is about the corners being there, not about a sample count.
        assertTrue("ring had ${ring.size} points", ring.size >= 4)
        assertEquals(200f, ring.minOf { it.x }, SLOP)
        assertEquals(600f, ring.maxOf { it.x }, SLOP)
        assertEquals(200f, ring.minOf { it.y }, SLOP)
        assertEquals(600f, ring.maxOf { it.y }, SLOP)
    }

    @Test
    fun aStraightDragInRingModeIsNotACapture() {
        // Measured on a phone before the area check existed: this passed every
        // size check and produced a blank picture, because a line encloses
        // nothing for the mask to keep.
        setUpSurface(lasso = true)

        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(200f, 200f))
            moveTo(Offset(400f, 400f))
            moveTo(Offset(700f, 700f))
            up()
        }

        assertEquals(emptyList<Pair<Rect, List<Offset>>>(), captured)
    }

    @Test
    fun aBoxReportsTheDraggedRectangleAndNoRing() {
        setUpSurface(lasso = false)

        // Stepped rather than three big jumps, because the box starts where the
        // touch slop is crossed rather than where the finger landed: a single
        // 280-pixel first move puts the corner 280 pixels in. A finger emits
        // events every few pixels, so the real error is the slop and no more —
        // and the marquee is drawn from the same corner, so what is dragged is
        // what is captured either way.
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(300f, 700f))
            for (step in 1..20) moveTo(Offset(300f + step * 20f, 700f - step * 20f))
            up()
        }

        assertEquals(1, captured.size)
        val (box, ring) = captured.single()
        // Dragged up and to the right, and still reported top-left first.
        assertEquals(300f, box.left, SLOP)
        assertEquals(300f, box.top, SLOP)
        assertEquals(700f, box.right, SLOP)
        assertEquals(700f, box.bottom, SLOP)
        assertEquals(emptyList<Offset>(), ring)
    }

    @Test
    fun aTapThatWanderedIsNotACapture() {
        setUpSurface(lasso = false)

        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(400f, 400f))
            moveTo(Offset(403f, 402f))
            up()
        }

        assertEquals(emptyList<Pair<Rect, List<Offset>>>(), captured)
    }

    private companion object {
        const val TAG = "capture-surface"

        /**
         * Room for the touch slop, which is where a drag is reported as starting.
         */
        const val SLOP = 30f
    }
}
