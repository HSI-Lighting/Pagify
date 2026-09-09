package com.hsilighting.pagify.ui.components

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.movedBy
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.sizeRange

/**
 * What has been drawn on a capture, and what it is being drawn with.
 *
 * **The same marks the reader makes, held somewhere a reader is not.** A
 * picture of a 3D model or a drawing has no document behind it — no pages, no
 * tiles, nothing to re-render from — but the thing somebody wants to do to it
 * is identical: circle a detail, put an arrow on it, write a note. The marks
 * themselves are already shared (`core.Markup`), and so is the editor that
 * draws them; this is the state between the two, which until now only the PDF
 * reader had.
 *
 * Held as a plain object rather than in a view model because it lives and dies
 * with one capture: a half-drawn arrow means nothing once the picture is gone.
 */
class CaptureMarkup {

    var marks by mutableStateOf<List<Markup>>(emptyList())
        private set

    var tool by mutableStateOf(MarkupTool.Pen)
        private set

    /**
     * Whether the tool is actually in hand.
     *
     * Separate from which tool it is, so putting it down and picking it up
     * returns the one you had, with its colour and its weight, instead of
     * starting again at the pen.
     */
    var armed by mutableStateOf(false)
        private set

    var colour by mutableStateOf(AnnotationColors.RED)
        private set

    var style by mutableStateOf(MarkupStyle.SOLID)
        private set

    /**
     * How heavy each tool draws, kept per tool.
     *
     * A highlighter and a pen do not want the same number, and one shared
     * setting means every switch between them is followed by a correction.
     */
    var sizes by mutableStateOf<Map<MarkupTool, Float>>(emptyMap())
        private set

    var selected by mutableStateOf<Int?>(null)
        private set

    var textFont by mutableStateOf(PdfFont.HELVETICA)
        private set

    var textSizePoints by mutableStateOf(14f)
        private set

    var textCurveDegrees by mutableStateOf(0f)
        private set

    /** What the tool in hand draws at. */
    val size: Float get() = sizes[tool] ?: tool.sizeRange.start

    fun use(which: MarkupTool) {
        tool = which
        armed = true
    }

    fun disarm() {
        armed = false
    }

    fun colour(value: Long) {
        colour = value
        // A colour chosen for a caption that is picked up changes that caption,
        // not only what the next one will be.
        recolourSelected(value)
    }

    fun style(value: MarkupStyle) {
        style = value
    }

    fun size(which: MarkupTool, value: Float) {
        sizes = sizes + (which to value.coerceIn(which.sizeRange))
    }

    fun font(value: PdfFont) {
        textFont = value
        rewriteSelected { it.copy(font = value) }
    }

    fun textSize(value: Float) {
        textSizePoints = value
        rewriteSelected { it.copy(sizePoints = value) }
    }

    fun textCurve(value: Float) {
        textCurveDegrees = value
        rewriteSelected { it.copy(curveDegrees = value) }
    }

    /** Add a mark that needed no recognition — a dragged shape, or a stroke. */
    fun add(shape: MarkupShape) {
        marks = marks + Markup(
            shape = shape,
            color = colour,
            widthPoints = size,
            style = style,
        )
    }

    fun undo() {
        marks = marks.dropLast(1)
        selected = null
    }

    fun clear() {
        marks = emptyList()
        selected = null
    }

    /**
     * Move the caption at [index] by [delta].
     *
     * By position rather than by identity, as the reader does it: the list *is*
     * the drawing, in the order it was made, and nothing reorders it — so the
     * index a drag started on is the mark it started on.
     */
    fun move(index: Int, delta: Offset) {
        if (delta == Offset.Zero) return
        val mark = marks.getOrNull(index) ?: return
        val shape = mark.shape as? MarkupShape.Text ?: return
        marks = marks.toMutableList().also { it[index] = mark.copy(shape = shape.movedBy(delta)) }
    }

    /** Pick a caption up, or put it down with a negative index. */
    fun select(index: Int) {
        val shape = marks.getOrNull(index)?.shape as? MarkupShape.Text
        selected = if (shape == null || index < 0) null else index
        if (shape != null) {
            textFont = shape.font
            textSizePoints = shape.sizePoints
            textCurveDegrees = shape.curveDegrees
        }
    }

    fun scaleSelected(factor: Float) {
        val index = selected ?: return
        val mark = marks.getOrNull(index) ?: return
        val shape = mark.shape as? MarkupShape.Text ?: return
        val grown = (shape.sizePoints * factor).coerceIn(MarkupTool.Text.sizeRange)
        textSizePoints = grown
        marks = marks.toMutableList().also {
            it[index] = mark.copy(shape = shape.copy(sizePoints = grown))
        }
    }

    /**
     * Rewrite a caption, or erase it.
     *
     * **Clearing the words is how one is deleted.** There is no separate delete:
     * a caption with nothing in it is nothing, and offering a second way to
     * remove it would be a control for something the keyboard already does.
     */
    fun rewrite(index: Int, text: String) {
        val mark = marks.getOrNull(index) ?: return
        val shape = mark.shape as? MarkupShape.Text ?: return
        marks = if (text.isBlank()) {
            selected = null
            marks.filterIndexed { at, _ -> at != index }
        } else {
            marks.toMutableList().also { it[index] = mark.copy(shape = shape.copy(text = text)) }
        }
    }

    fun erase(index: Int) {
        if (index !in marks.indices) return
        selected = null
        marks = marks.filterIndexed { at, _ -> at != index }
    }

    private fun recolourSelected(value: Long) {
        val index = selected ?: return
        val mark = marks.getOrNull(index) ?: return
        marks = marks.toMutableList().also { it[index] = mark.copy(color = value) }
    }

    private fun rewriteSelected(change: (MarkupShape.Text) -> MarkupShape.Text) {
        val index = selected ?: return
        val mark = marks.getOrNull(index) ?: return
        val shape = mark.shape as? MarkupShape.Text ?: return
        marks = marks.toMutableList().also { it[index] = mark.copy(shape = change(shape)) }
    }
}
