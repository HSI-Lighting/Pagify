package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin

/**
 * Turning a traced ring into a revision cloud.
 *
 * The failure that matters here is direction: scallops that bulge *inward* still
 * look like a cloud in a screenshot, just a slightly odd one, so nothing catches
 * it by eye. Every test below that could pass on an inward cloud is paired with
 * one that could not.
 */
class CloudStrokesTest {

    /** A square traced clockwise on screen, y downward. */
    private val square = listOf(
        Offset(100f, 100f),
        Offset(300f, 100f),
        Offset(300f, 300f),
        Offset(100f, 300f),
    )

    /** Width 2.4 gives a 19.2-point scallop; the square's perimeter is 800. */
    private val width = 2.4f

    /**
     * Whether a point is inside the traced ring, by crossing count.
     *
     * The whole cloud is judged with this rather than by distance from a centre,
     * because a centre is exactly what stops working on the concave shapes these
     * tests exist to cover.
     */
    private fun isInside(polygon: List<Offset>, point: Offset): Boolean {
        var inside = false
        for (index in polygon.indices) {
            val a = polygon[index]
            val b = polygon[(index + 1) % polygon.size]
            val straddles = (a.y > point.y) != (b.y > point.y)
            if (straddles) {
                val crossing = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
                if (crossing > point.x) inside = !inside
            }
        }
        return inside
    }

    /**
     * The farthest-out point of each scallop.
     *
     * Judged instead of the whole outline because at a convex corner the chord
     * cuts across the tip, so the first and last few points of that arc sit inside
     * the traced ring quite correctly — the cloud is hugging the corner. The apex
     * is the part that cannot: on an inward scallop every one of them is inside.
     */
    private fun apexes(cloud: List<Offset>): List<Offset> =
        cloud.filterIndexed { index, _ -> index % CLOUD_ARC_SEGMENTS == CLOUD_ARC_SEGMENTS / 2 }

    private fun perimeterOf(ring: List<Offset>): Float =
        (ring + ring.first()).zipWithNext()
            .sumOf { (a, b) -> hypot(b.x - a.x, b.y - a.y).toDouble() }
            .toFloat()

    @Test
    fun `a traced ring comes back as a closed outline`() {
        val cloud = cloudOutline(square, width)

        assertTrue("no outline", cloud.size > square.size)
        assertEquals("not closed", cloud.first(), cloud.last())
    }

    @Test
    fun `every scallop bulges outward`() {
        // The apex of each half-circle is the point farthest from its chord, so
        // if any arc were drawn the wrong way round its apex would land inside
        // the ring. Inward scallops are still a cloud to look at; they are only
        // wrong once you notice the shape has shrunk.
        val cloud = cloudOutline(square, width)
        val inward = apexes(cloud).count { isInside(square, it) }

        assertEquals("$inward of ${apexes(cloud).size} scallops pointed inward", 0, inward)
        // And the outline as a whole stays outside apart from the corners, which
        // is what stops a mostly-right cloud with a few inverted arcs passing.
        assertTrue(cloud.count { isInside(square, it) } < cloud.size / 10)
    }

    @Test
    fun `a ring traced the other way clouds the same way`() {
        // Nobody traces consistently, and half of all clouds would point inward if
        // this came from the winding without being corrected for it.
        val clockwise = cloudOutline(square, width)
        val anticlockwise = cloudOutline(square.reversed(), width)

        assertEquals(clockwise.size, anticlockwise.size)
        assertEquals(0, apexes(anticlockwise).count { isInside(square, it) })
    }

    @Test
    fun `scallops bulge away from the inside of a concave ring`() {
        // An L. Its centroid lies in the notch — outside the shape — so anything
        // that decided "outward" by pointing away from the middle would send the
        // scallops along the inner corner the wrong way.
        val ell = listOf(
            Offset(0f, 0f),
            Offset(300f, 0f),
            Offset(300f, 100f),
            Offset(100f, 100f),
            Offset(100f, 300f),
            Offset(0f, 300f),
        )
        val cloud = cloudOutline(ell, width)

        assertTrue("no outline", cloud.isNotEmpty())
        assertEquals(0, apexes(cloud).count { isInside(ell, it) })
    }

    @Test
    fun `the cloud follows the ring it was traced around`() {
        // Outward, but not by much: a scallop reaches half its chord beyond the
        // line, and anything further means the arcs are being built on the wrong
        // radius. Ringing a paragraph should not swallow the one below it.
        val cloud = cloudOutline(square, width)
        val reach = bumpLength(width) / 2f + 1f

        cloud.forEach { point ->
            val nearest = (square + square.first()).zipWithNext().minOf { (a, b) ->
                distanceToSegment(point, a, b).toDouble()
            }.toFloat()
            assertTrue("a scallop strayed ${nearest}pt from the ring", nearest <= reach)
        }
    }

    @Test
    fun `a heavier nib makes bigger scallops, and fewer of them`() {
        // The one size control, meaning what it always meant.
        val fine = cloudOutline(square, 1.2f)
        val heavy = cloudOutline(square, 5f)

        assertTrue(bumpLength(5f) > bumpLength(1.2f))
        assertTrue("fine=${fine.size} heavy=${heavy.size}", fine.size > heavy.size)
    }

    @Test
    fun `the scallops are evenly spaced whatever the hand did`() {
        // A ring traced slowly along one edge and quickly along the rest: hundreds
        // of samples in one place, a handful everywhere else. Spacing the bumps by
        // point rather than by length would spend most of the cloud on that edge.
        val crawled = (0..200).map { Offset(100f + it * 1f, 100f) }
        val rest = listOf(Offset(300f, 300f), Offset(100f, 300f))
        val cloud = cloudOutline(crawled + rest, width)

        // Every bump is a half circle on an equal chord, so every apex sits the
        // same distance from the ring. Uneven spacing shows up as uneven reach.
        val bumps = apexes(cloud)
        val reaches = bumps.map { point ->
            (crawled + rest).let { ring ->
                (ring + ring.first()).zipWithNext().minOf { (a, b) ->
                    distanceToSegment(point, a, b).toDouble()
                }.toFloat()
            }
        }
        // Corners aside, where a bump spans two edges, they should agree closely.
        val typical = reaches.sorted()[reaches.size / 2]
        assertTrue("reaches vary: ${reaches.map { it.toInt() }}", typical > 1f)
        assertTrue(reaches.count { it > typical * 2f } <= 3)
    }

    @Test
    fun `a tap is not a cloud`() {
        assertEquals(emptyList<Offset>(), cloudOutline(emptyList(), width))
        assertEquals(emptyList<Offset>(), cloudOutline(listOf(Offset(10f, 10f)), width))
        assertEquals(
            emptyList<Offset>(),
            cloudOutline(List(8) { Offset(10f, 10f) }, width),
        )
    }

    @Test
    fun `a ring too small for one scallop is left alone`() {
        // Rather than a single arc bent round onto itself, which is a circle with
        // a kink in it and reads as a mistake.
        val speck = (0 until 12).map { step ->
            val angle = step / 12f * 2f * PI.toFloat()
            Offset(50f + 1f * cos(angle), 50f + 1f * sin(angle))
        }
        assertEquals(emptyList<Offset>(), cloudOutline(speck, width))
    }

    @Test
    fun `a cloud can be dashed like any other line`() {
        // The line type applies to it as it does to every other drawn shape.
        val cloud = cloudOutline(square, width)
        val solid = dashed(cloud, MarkupStyle.SOLID, width)
        val broken = dashed(cloud, MarkupStyle.DASH_1, width)

        assertEquals(1, solid.size)
        assertTrue("only ${broken.size} pieces", broken.size > solid.size)
    }

    private fun distanceToSegment(point: Offset, a: Offset, b: Offset): Float {
        val span = b - a
        val lengthSquared = span.x * span.x + span.y * span.y
        if (lengthSquared <= 0f) return (point - a).getDistance()
        val t = (((point - a).x * span.x + (point - a).y * span.y) / lengthSquared)
            .coerceIn(0f, 1f)
        return (point - (a + span * t)).getDistance()
    }
}
