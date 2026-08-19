package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The rules of a markup drag, off-device.
 *
 * Compose gestures cannot be driven from a host test, which is why these rules
 * are in a plain class rather than in the pointer handler: the two that matter —
 * when a stroke snaps, and that nothing reaches the engine while a finger is down
 * — are the ones that would otherwise only be checkable by hand.
 */
class MarkupGestureTest {

    private fun stroke(gesture: MarkupGesture, points: List<Offset>) {
        gesture.down(points.first())
        points.drop(1).forEach(gesture::move)
    }

    private fun circlePoints(): List<Offset> = List(12) { i ->
        val t = i / 12f * 2f * Math.PI.toFloat()
        Offset(100f + 30f * kotlin.math.cos(t), 100f + 30f * kotlin.math.sin(t))
    }

    @Test
    fun `a stroke lifted straight away is kept exactly as drawn`() {
        // The anti-surprise rule. Nothing is recognised unless it was asked for.
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, circlePoints())

        val outcome = gesture.up()
        assertTrue("expected a plain commit, got $outcome", outcome is MarkupGesture.Outcome.Commit)
        assertTrue((outcome as MarkupGesture.Outcome.Commit).shape is MarkupShape.Freehand)
    }

    @Test
    fun `holding still at the end asks for recognition`() {
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, circlePoints())
        gesture.still()

        val outcome = gesture.up()
        assertTrue("expected recognition, got $outcome", outcome is MarkupGesture.Outcome.Recognise)
    }

    @Test
    fun `moving again after a hold cancels the snap`() {
        // A pause halfway through a long squiggle must not snap it — the hold has
        // to be the last thing that happened.
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, circlePoints())
        gesture.still()
        assertTrue(gesture.isDwelling)

        gesture.move(Offset(180f, 180f))
        assertFalse("the dwell survived real movement", gesture.isDwelling)
        assertTrue(gesture.up() is MarkupGesture.Outcome.Commit)
    }

    @Test
    fun `a finger resting on glass still counts as still`() {
        // A hand at rest jitters by a point or so. Treating that as movement means
        // the dwell never completes on a real device, only in a test.
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, circlePoints())
        gesture.still()
        gesture.move(circlePoints().last() + Offset(0.4f, 0.5f))

        assertTrue("a jitter cancelled the dwell", gesture.isDwelling)
        assertTrue(gesture.up() is MarkupGesture.Outcome.Recognise)
    }

    @Test
    fun `a dragged tool never asks for recognition`() {
        // There is nothing to recognise: the shape is defined by two corners, so a
        // hold before lifting should change nothing at all.
        for (tool in MarkupTool.entries.filter { it.isDragged }) {
            val gesture = MarkupGesture(tool)
            stroke(gesture, listOf(Offset(10f, 10f), Offset(90f, 70f)))
            gesture.still()

            val outcome = gesture.up()
            assertTrue(
                "$tool asked for recognition",
                outcome is MarkupGesture.Outcome.Commit,
            )
        }
    }

    @Test
    fun `a dragged tool builds its shape from the two ends`() {
        val gesture = MarkupGesture(MarkupTool.Arrow)
        stroke(gesture, listOf(Offset(10f, 10f), Offset(50f, 50f), Offset(90f, 70f)))

        val shape = (gesture.up() as MarkupGesture.Outcome.Commit).shape
        assertEquals(MarkupShape.Arrow(Offset(10f, 10f), Offset(90f, 70f)), shape)
    }

    @Test
    fun `a rectangle dragged backwards is still a rectangle`() {
        val gesture = MarkupGesture(MarkupTool.Rectangle)
        stroke(gesture, listOf(Offset(90f, 70f), Offset(10f, 10f)))

        val shape = (gesture.up() as MarkupGesture.Outcome.Commit).shape as MarkupShape.Rectangle
        assertEquals(10f, shape.rect.left)
        assertEquals(10f, shape.rect.top)
        assertEquals(90f, shape.rect.right)
        assertEquals(70f, shape.rect.bottom)
    }

    @Test
    fun `a tap that moved a couple of points is ignored`() {
        val gesture = MarkupGesture(MarkupTool.Rectangle)
        stroke(gesture, listOf(Offset(50f, 50f), Offset(51f, 51f)))
        assertEquals(MarkupGesture.Outcome.Nothing, gesture.up())
    }

    @Test
    fun `a tap with no movement at all produces nothing`() {
        val gesture = MarkupGesture(MarkupTool.Pen)
        gesture.down(Offset(50f, 50f))
        assertEquals(MarkupGesture.Outcome.Nothing, gesture.up())
    }

    @Test
    fun `too short a stroke cannot dwell into a shape`() {
        // Three points is not a shape, it is a guess.
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, listOf(Offset(0f, 0f), Offset(5f, 5f), Offset(10f, 0f)))
        gesture.still()

        assertFalse(gesture.isDwelling)
        assertTrue(gesture.up() is MarkupGesture.Outcome.Commit)
    }

    @Test
    fun `the preview follows the finger and disappears when it lifts`() {
        val gesture = MarkupGesture(MarkupTool.Ellipse)
        assertNull("there is nothing to draw before a touch", gesture.preview)

        gesture.down(Offset(10f, 10f))
        gesture.move(Offset(60f, 40f))
        assertEquals(
            MarkupShape.Ellipse(rectFromCorners(10f, 10f, 60f, 40f)),
            gesture.preview,
        )

        gesture.up()
        assertNull("the preview outlived the gesture", gesture.preview)
    }

    @Test
    fun `a cancelled gesture commits nothing`() {
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, circlePoints())
        gesture.still()
        gesture.cancel()

        assertNull(gesture.preview)
        assertEquals(MarkupGesture.Outcome.Nothing, gesture.up())
    }

    @Test
    fun `a second gesture does not inherit the first one's points`() {
        val gesture = MarkupGesture(MarkupTool.Pen)
        stroke(gesture, circlePoints())
        gesture.up()

        gesture.down(Offset(0f, 0f))
        gesture.move(Offset(20f, 0f))
        val shape = (gesture.up() as MarkupGesture.Outcome.Commit).shape as MarkupShape.Freehand
        assertEquals(2, shape.points.size)
    }
}
