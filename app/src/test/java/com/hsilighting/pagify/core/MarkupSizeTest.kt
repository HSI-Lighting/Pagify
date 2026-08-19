package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * How heavy a tool draws, and how that reaches the engine.
 *
 * The part worth pinning is the highlighter: its intensity rides in the colour's
 * **alpha**, because a wash and a stroke are the same `Markup` on the wire. That
 * is invisible in the type — a `Long` colour looks like a colour — so it is the
 * kind of thing that quietly stops working.
 */
class MarkupSizeTest {

    @Test
    fun `a stroke tool carries its size as a nib width`() {
        val mark = markupFor(
            MarkupShape.Line(Offset.Zero, Offset(10f, 10f)),
            MarkupTool.Line,
            color = 0xFFFF0000,
            size = 6f,
        )

        assertEquals(6f, mark.widthPoints)
        assertEquals(0xFFFF0000, mark.color)
    }

    @Test
    fun `the highlighter carries its intensity in the alpha channel`() {
        // Half intensity is alpha 127 over the same red, and no width at all: a
        // wash has no nib.
        val mark = markupFor(
            MarkupShape.Highlight(androidx.compose.ui.geometry.Rect(0f, 0f, 10f, 10f)),
            MarkupTool.Highlight,
            color = 0xFFFF0000,
            size = 0.5f,
        )

        assertEquals(0f, mark.widthPoints)
        assertEquals(127L, (mark.color shr 24) and 0xFF)
        assertEquals(0xFF0000L, mark.color and 0xFFFFFF)
    }

    @Test
    fun `a fully opaque intensity is still expressible, and the engine caps it`() {
        // Kotlin does not clamp: the ceiling belongs in one place, and that place
        // is the compositor, so the exported file and the preview agree.
        val mark = markupFor(
            MarkupShape.Highlight(androidx.compose.ui.geometry.Rect(0f, 0f, 10f, 10f)),
            MarkupTool.Highlight,
            color = 0xFF00FF00,
            size = 1f,
        )
        assertEquals(255L, (mark.color shr 24) and 0xFF)
    }

    @Test
    fun `an intensity outside nought to one is brought back into range`() {
        val over = markupFor(
            MarkupShape.Highlight(androidx.compose.ui.geometry.Rect(0f, 0f, 1f, 1f)),
            MarkupTool.Highlight,
            color = 0xFF0000FF,
            size = 4f,
        )
        assertEquals(255L, (over.color shr 24) and 0xFF)
    }

    @Test
    fun `every tool starts somewhere sensible`() {
        val sizes = defaultMarkupSizes()

        assertEquals(MarkupTool.entries.size, sizes.size)
        MarkupTool.entries.forEach { tool ->
            val size = sizes.getValue(tool)
            assertTrue(
                "$tool starts at $size, outside its own range ${tool.sizeRange}",
                size in tool.sizeRange,
            )
        }
    }

    @Test
    fun `every preset is inside the range its slider offers`() {
        // A preset the slider cannot reach would jump the moment it was touched.
        MarkupTool.entries.forEach { tool ->
            tool.sizePresets.forEach { preset ->
                assertTrue(
                    "$tool preset $preset is outside ${tool.sizeRange}",
                    preset in tool.sizeRange,
                )
            }
        }
    }

    @Test
    fun `presets are offered thinnest first`() {
        MarkupTool.entries.forEach { tool ->
            assertEquals(
                "$tool presets are out of order",
                tool.sizePresets.sorted(),
                tool.sizePresets,
            )
        }
    }

    @Test
    fun `only the highlighter is measured as an intensity`() {
        // Everything else is a width, and the difference decides both the range and
        // whether the size reaches the engine as alpha or as a nib.
        assertTrue(MarkupTool.Highlight.isIntensity)
        MarkupTool.entries.filter { it != MarkupTool.Highlight }.forEach {
            assertTrue("$it should be measured as a width", !it.isIntensity)
        }
    }

    @Test
    fun `an intensity range stops short of opaque and a width range stops short of zero`() {
        assertTrue(MarkupTool.Highlight.sizeRange.endInclusive < 1f)
        assertTrue(MarkupTool.Pen.sizeRange.start > 0f)
    }
}
