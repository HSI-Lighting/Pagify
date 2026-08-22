package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI
import kotlin.math.abs

/**
 * Laying text along a baseline.
 *
 * The failures here are the quiet sort: text that drifts a little further wrong
 * with every glyph, or that reads correctly on screen and lands somewhere else in
 * the file. Both come from measuring with one ruler and drawing with another, so
 * most of these pin the layout to the font's own numbers.
 */
class TextLayoutTest {

    private val font = PdfFont.HELVETICA
    private val size = 24f

    private fun straight(text: String) =
        layOutText(text, font, size, straightBaseline(Offset(100f, 200f), text, font, size))

    @Test
    fun `every glyph gets a place`() {
        assertEquals(5, straight("Hello").size)
        assertEquals("Hello".toList(), straight("Hello").map { it.character })
    }

    @Test
    fun `glyphs advance by the font's own widths, not by a guess`() {
        // Helvetica's H is 722 thousandths and its e is 556. Measuring with
        // anything else — the phone's own sans, a fixed fraction of the size —
        // drifts further wrong with every letter, and the drift only shows up
        // once the text is beside something it has to line up with.
        val placed = straight("He")

        assertEquals(100f, placed[0].origin.x, 0.01f)
        assertEquals(100f + 722f * size / 1000f, placed[1].origin.x, 0.01f)
    }

    @Test
    fun `a straight line of text stays on its baseline`() {
        val placed = straight("Level")

        placed.forEach {
            assertEquals(200f, it.origin.y, 0.01f)
            assertEquals(0f, it.radians, 0.001f)
        }
    }

    @Test
    fun `Courier gives every glyph the same room`() {
        val placed = layOutText(
            "Wil",
            PdfFont.COURIER,
            size,
            straightBaseline(Offset(0f, 0f), "Wil", PdfFont.COURIER, size),
        )
        val steps = placed.zipWithNext { a, b -> b.origin.x - a.origin.x }

        assertEquals(1, steps.map { it.toInt() }.distinct().size)
    }

    @Test
    fun `text on a curve leans with the curve`() {
        // A quarter turn: the text starts running right and ends running down, so
        // the last glyph must be turned about ninety degrees from the first.
        val quarter = (0..24).map { step ->
            val t = step / 24f * PI.toFloat() / 2f
            Offset(200f * kotlin.math.sin(t), 200f - 200f * kotlin.math.cos(t))
        }
        val placed = layOutText("Curving text", font, size, quarter)

        assertTrue("only ${placed.size} glyphs placed", placed.size > 6)
        val turn = placed.last().radians - placed.first().radians
        assertTrue("turned only ${Math.toDegrees(turn.toDouble())}°", turn > 0.5f)
    }

    @Test
    fun `every glyph on a curve sits on the curve`() {
        val ring = (0..40).map { step ->
            val t = step / 40f * 2f * PI.toFloat()
            Offset(300f + 150f * kotlin.math.cos(t), 300f + 150f * kotlin.math.sin(t))
        }
        val placed = layOutText("Around and around", font, size, ring)

        placed.forEach {
            val radius = (it.origin - Offset(300f, 300f)).getDistance()
            assertEquals("a glyph left the curve", 150f, radius, 2f)
        }
    }

    @Test
    fun `text longer than its baseline is cut, not bunched`() {
        // Bunching the overflow at the last point draws a smear of overlapping
        // letters, which reads as a bug. Running past the end puts text where no
        // line was drawn. Dropping what will not fit is the only honest answer.
        val shortLine = listOf(Offset(0f, 0f), Offset(40f, 0f))
        val placed = layOutText("Far too long for this", font, size, shortLine)

        assertTrue("nothing was placed", placed.isNotEmpty())
        assertTrue("nothing was dropped", placed.size < "Far too long for this".length)
        placed.forEach { assertTrue("a glyph ran past the end", it.origin.x <= 40.01f) }
    }

    @Test
    fun `a baseline is exactly as long as the text that sits on it`() {
        val text = "Measure me"
        val baseline = straightBaseline(Offset(10f, 10f), text, font, size)

        assertEquals(font.widthOf(text, size), baselineLength(baseline), 0.01f)
        // And so nothing is dropped from text placed by a tap.
        assertEquals(text.length, layOutText(text, font, size, baseline).size)
    }

    @Test
    fun `nothing is placed without text or a line to put it on`() {
        assertEquals(emptyList<GlyphPlacement>(), layOutText("", font, size, straight("x").map { it.origin }))
        assertEquals(emptyList<GlyphPlacement>(), layOutText("Hi", font, size, listOf(Offset.Zero)))
    }

    @Test
    fun `a bigger size takes proportionally more room`() {
        val small = font.widthOf("Proportional", 12f)
        val large = font.widthOf("Proportional", 24f)

        assertEquals(2f, large / small, 0.001f)
    }

    @Test
    fun `an unprintable character does not collapse the line`() {
        // A tab or an emoji has no entry in the tables. Taking a space's width
        // keeps everything after it where it belongs, where a zero would slide the
        // rest of the line back over itself.
        val withTab = font.widthOf("a\tb", size)
        val withSpace = font.widthOf("a b", size)

        assertEquals(withSpace, withTab, 0.01f)
        assertTrue(abs(withTab - font.widthOf("ab", size)) > 1f)
    }

    @Test
    fun `a baseline of one point lays out nothing at all`() {
        // Which is why a tap is expanded into a run before the words are stored.
        // Kept here as the reason: the screenshot editor stored the tap itself
        // once, and drew a frame round a caption with no letters in it.
        val tapped = layOutText("Check this", font, 12f, listOf(Offset(10f, 200f)))
        assertTrue("a point is not a baseline", tapped.isEmpty())

        val run = layOutText(
            "Check this",
            font,
            12f,
            straightBaseline(Offset(10f, 200f), "Check this", font, 12f),
        )
        assertEquals("Check this".length, run.size)
    }

    // ------------------------------------------------------ how big it may go --

    @Test
    fun `the biggest size is the one that still fits the width`() {
        val words = "As wide as the page"
        val page = 595f

        val biggest = font.sizeThatFits(words, page)
        assertEquals(page, font.widthOf(words, biggest), 0.5f)
        // A hair over and it no longer fits, which is what "biggest" means.
        assertTrue(font.widthOf(words, biggest + 1f) > page)
    }

    @Test
    fun `longer words get a smaller ceiling`() {
        // The ceiling is about the run, not the type: the same size that fits two
        // words runs off the sheet with ten.
        val page = 595f
        val short = font.sizeThatFits("Fix", page)
        val long = font.sizeThatFits("Fix this whole paragraph please", page)
        assertTrue("short=$short long=$long", short > long)
    }

    @Test
    fun `nothing to measure falls back rather than dividing by zero`() {
        assertEquals(MAXIMUM_TEXT_POINTS, font.sizeThatFits("", 595f), 0.01f)
        assertEquals(MAXIMUM_TEXT_POINTS, font.sizeThatFits("Fix", 0f), 0.01f)
    }

    // ------------------------------------------------------- generated arcs --

    @Test
    fun `no bend is a straight line`() {
        val straight = curvedBaseline(Offset(10f, 200f), "Bend me", font, 12f, degrees = 0f)
        assertEquals(straightBaseline(Offset(10f, 200f), "Bend me", font, 12f), straight)
    }

    @Test
    fun `the arc is exactly as long as the words`() {
        // Every letter has to land on it. An arc built on the straight width
        // would be short by the difference between a chord and its arc, and the
        // last letters would drop off the end.
        val words = "Bend me round"
        listOf(20f, 60f, 120f, 179f).forEach { degrees ->
            val arc = curvedBaseline(Offset(10f, 200f), words, font, 12f, degrees)
            assertEquals(
                "at $degrees degrees",
                font.widthOf(words, 12f),
                baselineLength(arc),
                0.5f,
            )
        }
    }

    @Test
    fun `every letter lands on the arc, at any bend`() {
        val words = "Bend me round"
        listOf(-150f, -45f, 45f, 150f).forEach { degrees ->
            val arc = curvedBaseline(Offset(10f, 200f), words, font, 12f, degrees)
            val placed = layOutText(words, font, 12f, arc)
            assertEquals("at $degrees degrees", words.length, placed.size)
        }
    }

    @Test
    fun `a positive bend arches up and a negative one sags`() {
        val words = "Bend me"
        val up = curvedBaseline(Offset(10f, 200f), words, font, 12f, degrees = 90f)
        val down = curvedBaseline(Offset(10f, 200f), words, font, 12f, degrees = -90f)

        // y runs downward, so arching up means the middle of the run is above
        // the ends — a smaller y than the anchor.
        assertTrue(up[up.size / 2].y < 200f)
        assertTrue(down[down.size / 2].y > 200f)
        // Both start where they were placed, so the bend does not also move the
        // words sideways out from under the finger.
        assertEquals(Offset(10f, 200f), up.first())
        assertEquals(Offset(10f, 200f), down.first())
    }

    @Test
    fun `the letters turn as the line turns`() {
        val words = "Bend me round"
        val arc = curvedBaseline(Offset(10f, 200f), words, font, 12f, degrees = 120f)
        val placed = layOutText(words, font, 12f, arc)

        val turned = abs(placed.last().radians - placed.first().radians)
        // Near the whole bend, allowing for the last glyph sitting a little short
        // of the very end of the line.
        assertTrue("only turned $turned radians", turned > 1.6f)
    }

    // --------------------------------------------------------- clouded text --

    private fun framed(
        words: String = "Revise this",
        points: Float = 12f,
        frame: TextFrame = TextFrame.Cloud,
    ) =
        Annotation.Text(
            id = 1L,
            pageIndex = 0,
            text = words,
            path = listOf(Offset(100f, 200f)),
            font = font,
            sizePoints = points,
            color = 0xFFFF0000,
            frame = frame,
        )

    @Test
    fun `the cloud encloses the words it is drawn round`() {
        val mark = framed()
        val box = mark.textFrameBounds()

        // The whole run, from the anchor to the end of the last glyph, with the
        // cap height above the baseline and the descender below it.
        assertTrue(box.left < 100f)
        assertTrue(box.right > 100f + font.widthOf(mark.text, mark.sizePoints))
        assertTrue(box.top < 200f - mark.sizePoints * 0.7f)
        assertTrue(box.bottom > 200f)
    }

    @Test
    fun `the cloud grows with the point size, so only one thing is chosen`() {
        val small = framed(points = 9f).textFrameBounds()
        val large = framed(points = 36f).textFrameBounds()

        // Four times the type, near enough four times the box — the margin is a
        // proportion of the size, which is what lets the tool ask one question.
        assertEquals(4f, large.height / small.height, 0.05f)
        assertTrue(large.width > small.width * 3f)
    }

    @Test
    fun `the scallops face outwards`() {
        val mark = framed()
        val box = mark.textFrameBounds()
        val ring = mark.textFrameOutline()
        assertTrue(ring.size > 8)

        // The apex of each bump is halfway through its arc. Every one of them has
        // to be outside the box: scallops turned inward are the failure this
        // notation has had before, and they eat the words.
        val apexes = ring.indices
            .filter { it % CLOUD_ARC_SEGMENTS == CLOUD_ARC_SEGMENTS / 2 }
            .map { ring[it] }
        assertTrue(apexes.isNotEmpty())
        apexes.forEach { apex ->
            assertTrue("apex $apex fell inside $box", !box.deflate(0.5f).contains(apex))
        }
    }

    @Test
    fun `even a short word gets enough bumps to read as a cloud`() {
        // Few, fat bumps round a small box give a thought bubble rather than a
        // revision cloud: the corners round away and the ring stops following the
        // words. Six is where it starts looking like the notation again.
        val ring = framed(words = "Fix", points = 12f).textFrameOutline()
        val bumps = ring.size / CLOUD_ARC_SEGMENTS
        assertTrue("only $bumps bumps", bumps >= 6)
    }

    @Test
    fun `a clouded mark is grabbable anywhere inside its cloud`() {
        val mark = framed()
        val box = mark.textFrameBounds()

        assertTrue(mark.isHitBy(box.center, tolerance = 1f))
        // Including the empty space above the words, which is inside the ring and
        // so is part of the mark as far as anyone looking at it is concerned.
        assertTrue(mark.isHitBy(Offset(box.center.x, box.top + 1f), tolerance = 1f))
        assertTrue(!mark.isHitBy(Offset(box.right + 40f, box.center.y), tolerance = 1f))
    }

    @Test
    fun `a box frame is the measured box itself`() {
        val mark = framed(frame = TextFrame.Box)
        val box = mark.textFrameBounds()
        val ring = mark.textFrameOutline()

        // Four corners and back to the first, so it closes.
        assertEquals(5, ring.size)
        assertEquals(Offset(box.left, box.top), ring.first())
        assertEquals(ring.first(), ring.last())
    }

    @Test
    fun `an ellipse frame goes round the words rather than through them`() {
        val mark = framed(frame = TextFrame.Ellipse)
        val box = mark.textFrameBounds()
        val ring = mark.textFrameOutline()
        assertTrue(ring.size > 32)

        // Every corner of the box inside the ring *as drawn* — tested against the
        // polyline itself, not against the arithmetic that made it, or this passes
        // just as happily when the ellipse is inscribed in the box and cuts the
        // first and last letters off.
        listOf(
            Offset(box.left, box.top),
            Offset(box.right, box.top),
            Offset(box.right, box.bottom),
            Offset(box.left, box.bottom),
        ).forEach { corner ->
            assertTrue("corner $corner is outside the ring", encloses(ring, corner))
        }
    }

    /** Ray casting: true when [point] is inside the closed polyline [ring]. */
    private fun encloses(ring: List<Offset>, point: Offset): Boolean {
        var inside = false
        for (i in ring.indices) {
            val a = ring[i]
            val b = ring[(i + 1) % ring.size]
            val straddles = (a.y > point.y) != (b.y > point.y)
            if (!straddles) continue
            val crossingX = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
            if (point.x < crossingX) inside = !inside
        }
        return inside
    }

    @Test
    fun `plain text has no frame at all`() {
        assertTrue(framed(frame = TextFrame.None).textFrameOutline().isEmpty())
    }
}
