package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect

/**
 * A mark the user has added on top of a page.
 *
 * Every annotation stores its geometry in **page points from the top-left**, the
 * same space [TextSegment] uses — never in screen pixels. That is what lets a
 * mark stay put when the page is zoomed, re-rendered at a different scale, or
 * redrawn after rotation: the view multiplies by the current render scale at
 * draw time and nothing needs migrating.
 *
 * These live only in memory for now. Writing them into the PDF itself is roadmap
 * phase 3 and is what `EditableDocument` in the Rust core exists for; keeping the
 * model in page space means that step is a serialisation change, not a rewrite.
 */
sealed interface Annotation {
    val id: Long
    val pageIndex: Int

    /** Text picked out with the highlighter, as one rect per line covered. */
    data class Highlight(
        override val id: Long,
        override val pageIndex: Int,
        val rects: List<Rect>,
        val color: Long,
    ) : Annotation

    /** A freehand stroke drawn with the marker. */
    data class Ink(
        override val id: Long,
        override val pageIndex: Int,
        val points: List<Offset>,
        val color: Long,
        /** Stroke width in page points, so it scales with the page. */
        val strokeWidth: Float,
    ) : Annotation

    /** A typed note anchored to a point on the page. */
    data class Note(
        override val id: Long,
        override val pageIndex: Int,
        val anchor: Offset,
        val text: String,
        val color: Long,
    ) : Annotation

    /** A drawn signature, stored as strokes so it stays sharp at any zoom. */
    data class Signature(
        override val id: Long,
        override val pageIndex: Int,
        val strokes: List<List<Offset>>,
        val bounds: Rect,
        val color: Long,
    ) : Annotation
}

/**
 * The annotations on an open document, with undo.
 *
 * Deliberately a plain in-memory store rather than something backed by the
 * engine: annotations are edited far more often than they are saved, and going
 * through JNI for every dragged point would put the render lock in the middle of
 * a gesture.
 */
class AnnotationStore {

    private val byPage = mutableMapOf<Int, MutableList<Annotation>>()
    private val undoStack = ArrayDeque<Long>()
    private var nextId = 1L

    fun add(annotation: Annotation) {
        byPage.getOrPut(annotation.pageIndex) { mutableListOf() }.add(annotation)
        undoStack.addLast(annotation.id)
    }

    fun forPage(pageIndex: Int): List<Annotation> = byPage[pageIndex].orEmpty()

    fun nextId(): Long = nextId++

    /** Removes the most recently added annotation. @return true if one was removed. */
    fun undo(): Boolean {
        val id = undoStack.removeLastOrNull() ?: return false
        byPage.values.forEach { page -> page.removeAll { it.id == id } }
        return true
    }

    val canUndo: Boolean get() = undoStack.isNotEmpty()

    val isEmpty: Boolean get() = byPage.values.all { it.isEmpty() }

    fun clear() {
        byPage.clear()
        undoStack.clear()
    }
}

/** The tool driving the bottom ribbon. */
enum class AnnotationTool {
    /** No tool: taps and drags scroll and zoom as usual. */
    None,
    Pen,
    Note,
    Signature,
    Snapshot,
}

/**
 * What the pen does. Long-pressing the pen swaps between these.
 *
 * Highlight is the default because it is the common case on a document you are
 * reading, and it is the one that needs the engine's text geometry; the marker is
 * free-drawing and needs nothing but the touch points.
 */
enum class PenMode { Highlight, Marker }

/** Palette offered when long-pressing the pen. */
object AnnotationColors {
    const val YELLOW = 0xFFFFE066L
    const val GREEN = 0xFF8CE99AL
    const val BLUE = 0xFF74C0FCL
    const val PINK = 0xFFFFA8C5L
    const val ORANGE = 0xFFFFC078L
    const val RED = 0xFFFF6B6BL

    val highlightPalette = listOf(YELLOW, GREEN, BLUE, PINK, ORANGE, RED)

    /** Marker ink is drawn opaque, so it needs stronger, darker colours. */
    val markerPalette = listOf(
        0xFFE03131L, // red
        0xFF1971C2L, // blue
        0xFF2F9E44L, // green
        0xFFF08C00L, // amber
        0xFF212529L, // near-black
        0xFF9C36B5L, // purple
    )
}
