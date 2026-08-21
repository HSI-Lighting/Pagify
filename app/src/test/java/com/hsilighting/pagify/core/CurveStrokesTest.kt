package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.hypot

/**
 * Correcting a traced curve to the one it was meant to be.
 *
 * The failure that matters is the side: a curve that bows the wrong way is still a
 * smooth curve, so it looks right in isolation and is only wrong next to the thing
 * it was drawn around. Half the tests below exist to pin the direction.
 */
class CurveStrokesTest {

    private val from = Offset(0f, 0f)
    private val to = Offset(200f, 0f)

    /** A trace from [from] to [to] that bows [depth] in y at the middle. */
    private fun bowed(depth: Float, samples: Int = 20): List<Offset> =
        (0..samples).map { step ->
            val t = step.toFloat() / samples
            // A parabola through the ends, deepest in the middle, with a little
            // hand-shake on top so this is a trace and not already a curve.
            val shake = if (step % 2 == 0) 1.2f else -1.2f
            Offset(t * 200f, 4f * depth * t * (1f - t) + shake)
        }

    /** How far a point lies across the line from [from] to [to]; signed. */
    private fun across(point: Offset): Float {
        val span = to - from
        return ((point - from).x * span.y - (point - from).y * span.x) / span.getDistance()
    }

    /** The sharpest corner anywhere along a polyline, in degrees. */
    private fun sharpestCorner(curve: List<Offset>): Double =
        curve.zipWithNext().zipWithNext().maxOf { (a, b) ->
            val first = a.second - a.first
            val second = b.second - b.first
            if (first.getDistance() == 0f || second.getDistance() == 0f) {
                0.0
            } else {
                val turn = kotlin.math.atan2(second.y.toDouble(), second.x.toDouble()) -
                    kotlin.math.atan2(first.y.toDouble(), first.x.toDouble())
                abs(Math.toDegrees(kotlin.math.atan2(kotlin.math.sin(turn), kotlin.math.cos(turn))))
            }
        }

    @Test
    fun `a traced curve comes back smooth`() {
        // Smooth means no corners. The shake is gone, and so is any kink from how
        // the curve was put together.
        val curve = curveThrough(bowed(40f))

        assertTrue("only ${curve.size} points", curve.size > 20)
        assertTrue("a ${sharpestCorner(curve)}° corner", sharpestCorner(curve) < 20.0)
    }

    @Test
    fun `there is no corner where one bend becomes the next`() {
        // The bug this exists for: fitting each bend as its own arc and joining
        // them end to end leaves the two arriving and leaving at different angles,
        // so an S had a visible corner in the middle of it.
        val ess = (0..60).map { step ->
            val t = step.toFloat() / 60f
            val shake = if (step % 2 == 0) 1.5f else -1.5f
            Offset(t * 240f, (kotlin.math.sin(t * 2f * Math.PI.toFloat()) * 40f) + shake)
        }
        val curve = curveThrough(ess)

        assertTrue("a ${sharpestCorner(curve)}° corner at the join", sharpestCorner(curve) < 25.0)
    }

    @Test
    fun `it starts and ends where the trace did`() {
        val curve = curveThrough(bowed(40f))

        assertEquals(0f, (curve.first() - from).getDistance(), 2f)
        assertEquals(0f, (curve.last() - to).getDistance(), 2f)
    }

    @Test
    fun `it bows to the side the trace bowed to`() {
        // The one that cannot be seen in isolation: a curve bent the wrong way is
        // still a perfectly good curve, and only wrong beside what it was drawn
        // around.
        val downward = curveThrough(bowed(40f))
        val upward = curveThrough(bowed(-40f))

        val downMiddle = across(downward[downward.size / 2])
        val upMiddle = across(upward[upward.size / 2])

        assertTrue("bowed the wrong way: $downMiddle", downMiddle * across(Offset(100f, 40f)) > 0f)
        assertTrue("bowed the wrong way: $upMiddle", upMiddle * across(Offset(100f, -40f)) > 0f)
    }

    @Test
    fun `it bows about as far as the trace did`() {
        // Not a fixed bend: a shallow sweep and a deep one are different marks.
        val shallow = curveThrough(bowed(20f))
        val deep = curveThrough(bowed(80f))

        val shallowest = shallow.maxOf { abs(across(it)) }
        val deepest = deep.maxOf { abs(across(it)) }

        assertEquals(20f, shallowest, 4f)
        assertEquals(80f, deepest, 4f)
    }

    @Test
    fun `an almost straight trace is left straight`() {
        // Someone who reached for the curve tool and drew a straight line meant a
        // straight line. Bending it to make the tool feel used is the app arguing.
        val trace = bowed(0.5f)
        val curve = curveThrough(trace)

        // Two points — the trace's own ends, shake and all, not the ideal ones:
        // the correction is to a straight line, not to a line of my choosing.
        assertEquals(listOf(trace.first(), trace.last()), curve)
    }

    @Test
    fun `a simple arc stays one bend`() {
        // The hand shakes at every sample, so the direction of the turn flips
        // constantly in the raw points. None of that is a change of direction, and
        // a plain arc that came back as forty little bends would be useless.
        val curve = curveThrough(bowed(50f))

        val sides = curve.map { across(it) }.filter { abs(it) > 1f }.map { it > 0f }
        assertEquals("a single arc was cut into bends", 1, sides.distinct().size)
    }

    @Test
    fun `an S keeps both of its bends`() {
        // Changing direction while drawing is how you say "and then it goes the
        // other way". Flattening this into one arc throws away half of what was
        // drawn — which is what it used to do.
        val ess = (0..60).map { step ->
            val t = step.toFloat() / 60f
            // One hump up, one hump down, with hand-shake on top.
            val shake = if (step % 2 == 0) 1.5f else -1.5f
            Offset(t * 240f, (kotlin.math.sin(t * 2f * Math.PI.toFloat()) * 40f) + shake)
        }
        val curve = curveThrough(ess)

        val sides = curve.map { across(it) }.filter { abs(it) > 5f }.map { it > 0f }
        assertEquals("the S was flattened to one bend", 2, sides.distinct().size)
        // And it still runs end to end.
        assertEquals(0f, (curve.first() - ess.first()).getDistance(), 3f)
        assertEquals(0f, (curve.last() - ess.last()).getDistance(), 3f)
    }

    @Test
    fun `a tap is not a curve`() {
        assertEquals(emptyList<Offset>(), curveThrough(emptyList()))
        assertEquals(emptyList<Offset>(), curveThrough(listOf(Offset(5f, 5f))))
        assertEquals(emptyList<Offset>(), curveThrough(List(6) { Offset(5f, 5f) }))
    }

    @Test
    fun `a curved arrow is the curve and two barbs`() {
        val strokes = curvedArrowStrokes(bowed(40f), widthPoints = 3f)

        assertEquals(3, strokes.size)
        assertEquals(CURVE_SEGMENTS + 1, strokes.first().size)
        // Both barbs start at the tip — that is what keeps the point sharp.
        assertTrue(strokes.drop(1).all { (it.first() - strokes.first().last()).getDistance() < 0.01f })
    }

    @Test
    fun `the head points along the curve, not along the chord`() {
        // A head aligned to the straight line between the ends sits visibly askew
        // on anything but the shallowest bend.
        val strokes = curvedArrowStrokes(bowed(80f), widthPoints = 3f)
        val curve = strokes.first()

        val arrival = curve.last() - curve[curve.size - 2]
        val chord = to - from

        val arrivalAngle = kotlin.math.atan2(arrival.y.toDouble(), arrival.x.toDouble())
        val chordAngle = kotlin.math.atan2(chord.y.toDouble(), chord.x.toDouble())
        assertTrue(
            "the head followed the chord",
            abs(arrivalAngle - chordAngle) > 0.2,
        )

        // And the barbs open backwards from the tip, around the arrival direction.
        strokes.drop(1).forEach { barb ->
            val back = barb.last() - barb.first()
            assertTrue("a barb points forwards", back.x * arrival.x + back.y * arrival.y < 0f)
        }
    }

    @Test
    fun `a curve can be dashed like any other line`() {
        val curve = curveThrough(bowed(40f))
        val solid = dashed(curve, MarkupStyle.SOLID, 2f)
        val broken = dashed(curve, MarkupStyle.DASH_1, 2f)

        assertEquals(1, solid.size)
        assertTrue("only ${broken.size} pieces", broken.size > solid.size)
    }

    @Test
    fun `the curve stays the length of the trace, near enough`() {
        // A bow that overshot would swing the line well past where it was drawn.
        val curve = curveThrough(bowed(40f))
        val length = curve.zipWithNext()
            .sumOf { (a, b) -> hypot(b.x - a.x, b.y - a.y).toDouble() }
            .toFloat()

        // A shallow arc is a little longer than its chord, never twice as long.
        assertTrue("length $length", length > 200f && length < 400f)
    }
}
