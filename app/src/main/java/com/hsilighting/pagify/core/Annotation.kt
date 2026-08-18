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
 * True when [point] lands on this mark, allowing [tolerance] either side.
 *
 * Both are in page points. The tolerance is what makes the eraser usable at all:
 * a highlight is a few points tall and an ink stroke is a line with no area, so
 * an exact hit test would demand precision no finger has. Callers pass a fixed
 * touch radius converted through the current render scale, which keeps the target
 * the same physical size whatever the page is magnified to.
 */
fun Annotation.isHitBy(point: Offset, tolerance: Float): Boolean = when (this) {
    is Annotation.Highlight -> rects.any { it.inflate(tolerance).contains(point) }
    // Half the nib counts as part of the stroke — the ink you can see is what you
    // expect to be able to rub out.
    is Annotation.Ink -> points.isNear(point, tolerance + strokeWidth / 2f)
    is Annotation.Signature -> strokes.any { it.isNear(point, tolerance) }
    is Annotation.Note -> (point - anchor).getDistance() <= tolerance + NOTE_MARKER_RADIUS_POINTS
}

/** Radius of a note's marker dot, in page points. Shared with the layer that draws it. */
const val NOTE_MARKER_RADIUS_POINTS = 7f

/** True when [point] is within [tolerance] of any part of this polyline. */
private fun List<Offset>.isNear(point: Offset, tolerance: Float): Boolean {
    if (isEmpty()) return false
    if (size == 1) return (this[0] - point).getDistance() <= tolerance
    for (i in 1 until size) {
        if (distanceToSegment(point, this[i - 1], this[i]) <= tolerance) return true
    }
    return false
}

/** Shortest distance from [p] to the line segment [a]–[b]. */
private fun distanceToSegment(p: Offset, a: Offset, b: Offset): Float {
    val ab = b - a
    val lengthSquared = ab.x * ab.x + ab.y * ab.y
    // A zero-length segment is a point; projecting onto it would divide by zero.
    if (lengthSquared == 0f) return (p - a).getDistance()
    val ap = p - a
    val t = ((ap.x * ab.x + ap.y * ab.y) / lengthSquared).coerceIn(0f, 1f)
    return (p - (a + ab * t)).getDistance()
}

/** An annotation together with the position it occupied on its page. */
data class PlacedAnnotation(val index: Int, val annotation: Annotation)

/**
 * One reversible change to the annotation set.
 *
 * Every operation — adding a mark, erasing one, clearing a page, clearing the
 * document — is expressed as this single shape, so undo and redo have exactly one
 * thing to know how to reverse. It is deliberately the shape of the `Command` the
 * engine's roadmap calls for, so that when annotations start being written into
 * the PDF this becomes a serialisation change rather than a rewrite.
 */
data class AnnotationEdit(
    /** What to call this in the UI. */
    val label: String,
    /**
     * Marks this edit took away, each with the index it held **at the moment it
     * was taken**.
     *
     * Reversing walks the list backwards, re-inserting each at its recorded
     * index. That order is not a detail: removing one mark shifts the position of
     * everything after it, so the second recorded index is only meaningful in the
     * state the first removal left behind. Undoing them in the order they
     * happened puts marks back in the wrong places.
     */
    val removed: List<PlacedAnnotation> = emptyList(),
    val added: List<Annotation> = emptyList(),
) {
    /** The page this edit touched, so undo can take the reader back to see it. */
    val pageIndex: Int?
        get() = added.firstOrNull()?.pageIndex ?: removed.firstOrNull()?.annotation?.pageIndex

    /** How many marks this edit affected, for the wording of an undo message. */
    val size: Int get() = added.size + removed.size
}

/**
 * The annotations on an open document, with undo and redo.
 *
 * Deliberately a plain in-memory store rather than something backed by the
 * engine: annotations are edited far more often than they are saved, and going
 * through JNI for every dragged point would put the render lock in the middle of
 * a gesture.
 *
 * History is a list of [AnnotationEdit]s rather than a list of added ids. The
 * previous version could only pop the most recent *addition*, which is enough for
 * "undo the last mark" and cannot express deletion at all — so an eraser had
 * nothing to undo into.
 */
class AnnotationStore {

    private val byPage = mutableMapOf<Int, MutableList<Annotation>>()
    private val done = ArrayDeque<AnnotationEdit>()
    private val undone = ArrayDeque<AnnotationEdit>()
    private var nextId = 1L

    /** Removals gathering inside a single eraser stroke, if one is in progress. */
    private var openErase: MutableList<PlacedAnnotation>? = null

    fun nextId(): Long = nextId++

    fun forPage(pageIndex: Int): List<Annotation> = byPage[pageIndex].orEmpty()

    fun countOnPage(pageIndex: Int): Int = byPage[pageIndex]?.size ?: 0

    val total: Int get() = byPage.values.sumOf { it.size }

    val isEmpty: Boolean get() = total == 0

    val canUndo: Boolean get() = done.isNotEmpty()

    val canRedo: Boolean get() = undone.isNotEmpty()

    fun add(annotation: Annotation) {
        pageOf(annotation.pageIndex).add(annotation)
        record(AnnotationEdit(label = "mark", added = listOf(annotation)))
    }

    // ---------------------------------------------------------------- eraser --

    /**
     * Open an eraser stroke.
     *
     * Everything rubbed out before [endErase] becomes one undo step. Recording
     * each mark separately would be simpler, but then wiping five highlights in
     * one sweep would take five taps of undo to put back, which is not what the
     * gesture felt like.
     */
    fun beginErase() {
        openErase = mutableListOf()
    }

    /**
     * Remove the mark under [point], if any.
     *
     * @return true when something was taken, so the caller knows to redraw.
     */
    fun eraseAt(pageIndex: Int, point: Offset, tolerance: Float): Boolean {
        val page = byPage[pageIndex] ?: return false
        // Topmost first: later marks are drawn over earlier ones, so the last
        // match is the one under the finger.
        val index = page.indexOfLast { it.isHitBy(point, tolerance) }
        if (index < 0) return false

        val taken = PlacedAnnotation(index, page.removeAt(index))
        val open = openErase
        if (open != null) {
            open += taken
        } else {
            record(AnnotationEdit(label = "erase", removed = listOf(taken)))
        }
        return true
    }

    /** Close the stroke, committing everything it rubbed out as one step. */
    fun endErase() {
        val open = openErase ?: return
        openErase = null
        if (open.isNotEmpty()) {
            record(AnnotationEdit(label = "erase", removed = open))
        }
    }

    // ----------------------------------------------------------------- clear --

    /** @return how many marks were cleared. */
    fun clearPage(pageIndex: Int): Int = removeAll(label = "clear page") {
        it.pageIndex == pageIndex
    }

    /** @return how many marks were cleared. */
    fun clearAll(): Int = removeAll(label = "clear all") { true }

    private fun removeAll(label: String, predicate: (Annotation) -> Boolean): Int {
        val taken = mutableListOf<PlacedAnnotation>()
        byPage.values.forEach { page ->
            // Backwards, so each recorded index is still valid as the list
            // shrinks — the same invariant the eraser relies on.
            for (i in page.indices.reversed()) {
                if (predicate(page[i])) taken += PlacedAnnotation(i, page.removeAt(i))
            }
        }
        if (taken.isNotEmpty()) record(AnnotationEdit(label = label, removed = taken))
        return taken.size
    }

    // --------------------------------------------------------- undo and redo --

    /** @return the edit that was reversed, or null if there was nothing to undo. */
    fun undo(): AnnotationEdit? {
        val edit = done.removeLastOrNull() ?: return null
        edit.added.forEach { annotation ->
            pageOf(annotation.pageIndex).removeAll { it.id == annotation.id }
        }
        edit.removed.asReversed().forEach { (index, annotation) ->
            val page = pageOf(annotation.pageIndex)
            page.add(index.coerceIn(0, page.size), annotation)
        }
        undone.addLast(edit)
        return edit
    }

    /** @return the edit that was reapplied, or null if there was nothing to redo. */
    fun redo(): AnnotationEdit? {
        val edit = undone.removeLastOrNull() ?: return null
        // Forwards, reproducing the original sequence exactly, so the indices
        // recorded against it stay correct for the next undo.
        edit.removed.forEach { (_, annotation) ->
            pageOf(annotation.pageIndex).removeAll { it.id == annotation.id }
        }
        edit.added.forEach { pageOf(it.pageIndex).add(it) }
        done.addLast(edit)
        return edit
    }

    fun clear() {
        byPage.clear()
        done.clear()
        undone.clear()
        openErase = null
    }

    private fun pageOf(pageIndex: Int) = byPage.getOrPut(pageIndex) { mutableListOf() }

    private fun record(edit: AnnotationEdit) {
        done.addLast(edit)
        // A new edit makes the undone branch unreachable: there is no longer a
        // history in which those changes come next.
        undone.clear()
        while (done.size > HISTORY_LIMIT) done.removeFirst()
    }

    private companion object {
        /**
         * Edits kept. Deep enough that nobody reaches the end of it by hand, and
         * bounded so a long session cannot grow the history without limit.
         */
        const val HISTORY_LIMIT = 200
    }
}

/** The tool driving the bottom ribbon. */
enum class AnnotationTool {
    /** No tool: taps and drags scroll and zoom as usual. */
    None,
    Pen,
    Note,
    Signature,
    /** Rubs out whole marks. Tap one, or sweep across several. */
    Eraser,
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
