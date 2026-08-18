package com.hsilighting.pagify.core

import kotlin.math.abs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The numbers here are the shape of real gestures on the test tablet: a pinch
 * changes the finger separation by hundreds of pixels, while two fingers dragging
 * in parallel change it by single digits over a scroll of several hundred.
 */
class TwoFingerGestureTest {

    private val slop = 40f // 20.dp at this device's density

    private fun classify(
        spreadChange: Float,
        centroidTravel: Float,
        from: TwoFingerGesture = TwoFingerGesture.Undecided,
    ) = TwoFingerGesture.classify(from, spreadChange, centroidTravel, slop)

    @Test
    fun `a two-finger scroll is a pan, not a zoom`() {
        // Fingers wobble a few pixels apart over a 300 px scroll.
        assertEquals(TwoFingerGesture.Pan, classify(spreadChange = 6f, centroidTravel = 300f))
    }

    @Test
    fun `a pinch is a zoom`() {
        assertEquals(TwoFingerGesture.Zoom, classify(spreadChange = 220f, centroidTravel = 30f))
    }

    @Test
    fun `pinching in is a zoom too`() {
        assertEquals(TwoFingerGesture.Zoom, classify(spreadChange = -180f, centroidTravel = 25f))
    }

    @Test
    fun `a pinch anchored on one finger is a zoom, not a pan`() {
        // One finger still, the other moving: the midpoint travels about half as
        // far as the separation changes, so both are past the slop and the tie
        // has to fall to zoom.
        assertEquals(TwoFingerGesture.Zoom, classify(spreadChange = 200f, centroidTravel = 100f))
    }

    @Test
    fun `nothing is decided until something moves past the slop`() {
        assertEquals(TwoFingerGesture.Undecided, classify(spreadChange = 5f, centroidTravel = 12f))
    }

    @Test
    fun `a pan that drifts apart at the end stays a pan`() {
        // The whole point of holding the decision: fingers spreading late in a
        // scroll must not start zooming the page under them.
        assertEquals(
            TwoFingerGesture.Pan,
            classify(spreadChange = 500f, centroidTravel = 600f, from = TwoFingerGesture.Pan),
        )
    }

    @Test
    fun `a zoom stays a zoom once the fingers start travelling`() {
        assertEquals(
            TwoFingerGesture.Zoom,
            classify(spreadChange = 60f, centroidTravel = 900f, from = TwoFingerGesture.Zoom),
        )
    }

    /**
     * The accumulator half of the same bug.
     *
     * Symmetric noise must not accumulate. Clamping each step at 1.0 — as the
     * previous code did — makes 1.0 a reflecting barrier: the downward half of
     * the noise is truncated while the upward half survives, so the running total
     * drifts up until it crosses the handover threshold and zooms on its own.
     */
    @Test
    fun `the wobble of a two-finger scroll does not accumulate`() {
        var progress = 1f
        // 600 events, alternating equal-and-opposite factors: exactly what a
        // parallel drag delivers, and a product that should stay at 1.
        repeat(300) {
            progress = pinchProgressAfter(progress, 1.01f)
            progress = pinchProgressAfter(progress, 1f / 1.01f)
        }
        assertEquals(1f, progress, 0.001f)
        assertTrue(
            "drifted to $progress, past the ${PINCH_HANDOVER_FOR_TEST}x handover",
            progress < PINCH_HANDOVER_FOR_TEST,
        )
    }

    @Test
    fun `a real pinch still accumulates to the handover`() {
        var progress = 1f
        repeat(20) { progress = pinchProgressAfter(progress, 1.02f) }
        assertTrue("a deliberate pinch must still hand over, got $progress", progress > PINCH_HANDOVER_FOR_TEST)
    }

    @Test
    fun `pinching out is allowed to go below one while the gesture runs`() {
        // It must not be floored on the way in; the reader clamps only the value
        // it hands to the magnified view.
        var progress = 1f
        repeat(10) { progress = pinchProgressAfter(progress, 0.95f) }
        assertTrue("expected progress below 1, got $progress", progress < 1f)
        assertTrue(abs(progress - 0.598f) < 0.01f)
    }

    private companion object {
        /** Mirrors `PINCH_HANDOVER_ZOOM` in the reader. */
        const val PINCH_HANDOVER_FOR_TEST = 1.12f
    }
}
