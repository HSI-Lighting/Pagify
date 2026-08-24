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

    /**
     * A line, an arrow, a box or a circle, as the strokes it is made of.
     *
     * Several strokes rather than one because an arrow has three, and a dashed
     * anything has as many as it has dashes — the line type is baked in when the
     * shape is committed, since a dash array on an ink annotation is a thing most
     * viewers ignore.
     *
     * Saved into the PDF as ink, which is what a signature is too: every viewer
     * draws it correctly, and the app carries one mechanism rather than two.
     */
    data class Shape(
        override val id: Long,
        override val pageIndex: Int,
        val strokes: List<List<Offset>>,
        val color: Long,
        /** Stroke width in page points, so it scales with the page. */
        val strokeWidth: Float,
    ) : Annotation

    /**
     * Words written onto the page, straight or along a traced curve.
     *
     * Held as the string and the baseline it sits on rather than as the glyphs it
     * will become, because it is still text right up until it is saved: the size
     * can change, the font can change, and none of that should mean re-deriving
     * shapes. [layOutText] turns it into glyphs wherever glyphs are needed.
     *
     * A straight line and a curve are the same thing here — a straight baseline is
     * a [path] of two points — so nothing downstream needs two cases.
     */
    data class Text(
        override val id: Long,
        override val pageIndex: Int,
        val text: String,
        /** The baseline, in page points. Two points for straight text. */
        val path: List<Offset>,
        val font: PdfFont,
        /** Point size, as a printer means it. */
        val sizePoints: Float,
        val color: Long,
        /**
         * What is drawn around the words, if anything.
         *
         * Carried on the text rather than kept as a second mark beside it: the
         * frame is measured from the words every time it is drawn, so it cannot
         * drift out of step with them, and moving or rubbing out the text takes
         * it along.
         */
        val frame: TextFrame = TextFrame.None,
        /**
         * How far the baseline turns from end to end, in degrees.
         *
         * Kept on the mark rather than worked back out of the path, because a
         * caption that can be edited has to be able to say what it *is*: the
         * ribbon shows this number while the caption is selected, and rebuilding
         * the line at a new size or in a new face needs it.
         */
        val curveDegrees: Float = 0f,
    ) : Annotation {
        /** Whether it follows a bent line rather than a straight one. */
        val isCurved: Boolean get() = path.size > 2
    }

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
    // Every dash of a dashed shape is a stroke of its own, so a tap anywhere
    // along the shape finds it — including in a gap, within the tolerance.
    is Annotation.Shape -> strokes.any { it.isNear(point, tolerance + strokeWidth / 2f) }
    is Annotation.Note -> (point - anchor).getDistance() <= tolerance + NOTE_MARKER_RADIUS_POINTS
    // Along the baseline, allowing the height of the letters either side of it:
    // the baseline runs under the text, so a tap on the words themselves is above
    // it and would otherwise miss everything.
    //
    // Straight text keeps only the point it was tapped at, and the words run off
    // to the right of it. Testing that one point would make the mark grabbable by
    // its first letter and nowhere else, so the run is measured out to its end.
    // A framed mark is grabbable anywhere inside its frame, which is what it
    // looks like: a shape with words in it, not a line of words.
    is Annotation.Text -> when {
        frame != TextFrame.None -> textFrameBounds().inflate(tolerance).contains(point)
        // A block is grabbable anywhere over the words, which is what it looks
        // like. Testing only the first line's baseline would leave every line
        // below it untouchable.
        text.contains('\n') -> textBlockBounds().inflate(tolerance).contains(point)
        else -> textBaseline().isNear(point, tolerance + sizePoints)
    }
}

/**
 * The baseline as a line with length: the stored path for curved text, and the
 * measured extent of the run for straight text.
 */
internal fun Annotation.Text.textBaseline(): List<Offset> {
    if (isCurved || path.isEmpty()) return path
    val start = path.first()
    return listOf(start, start + Offset(font.widthOf(text, sizePoints), 0f))
}

/**
 * The same mark, shifted by [delta] page points.
 *
 * Every kind moves, so that dragging one is a property of a mark rather than of
 * the one tool that happens to offer it today. The id is kept: this is the same
 * mark in a new place, which is what lets undo put it back instead of leaving a
 * duplicate behind.
 */
fun Annotation.movedBy(delta: Offset): Annotation = when (this) {
    is Annotation.Highlight -> copy(rects = rects.map { it.translate(delta) })
    is Annotation.Ink -> copy(points = points.map { it + delta })
    is Annotation.Shape -> copy(strokes = strokes.map { stroke -> stroke.map { it + delta } })
    is Annotation.Signature -> copy(
        strokes = strokes.map { stroke -> stroke.map { it + delta } },
        bounds = bounds.translate(delta.x, delta.y),
    )
    is Annotation.Note -> copy(anchor = anchor + delta)
    is Annotation.Text -> copy(path = path.map { it + delta })
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

    /**
     * Every page that carries a mark, lowest first.
     *
     * Ascending so a save writes marks in page order, which makes the resulting
     * file's annotation arrays match the order a reader would expect — and makes
     * a partial save, if one ever fails halfway, stop at a page boundary rather
     * than somewhere arbitrary.
     */
    fun pagesWithMarks(): List<Int> =
        byPage.filterValues { it.isNotEmpty() }.keys.sorted()

    fun countOnPage(pageIndex: Int): Int = byPage[pageIndex]?.size ?: 0

    val total: Int get() = byPage.values.sumOf { it.size }

    val isEmpty: Boolean get() = total == 0

    val canUndo: Boolean get() = done.isNotEmpty()

    val canRedo: Boolean get() = undone.isNotEmpty()

    /**
     * Put a mark in without recording it in the history.
     *
     * For marks read back out of the document: they were not made in this session,
     * so "undo" should not reach back past the file the reader opened and start
     * removing things that were already in it. Their removal is undoable through
     * the *document's* history instead, which is where a change to the file
     * belongs.
     */
    fun addFromDocument(annotation: Annotation) {
        pageOf(annotation.pageIndex).add(annotation)
    }

    /** True if a mark with this id is still present on its page. */
    fun contains(pageIndex: Int, id: Long): Boolean =
        byPage[pageIndex]?.any { it.id == id } == true

    /**
     * Note that [id] is in use, so no later mark is given the same one.
     *
     * Marks read out of a file keep the ids they were written with — that is how
     * a caption saved yesterday is still the same caption today — and those ids
     * were handed out by an earlier run of this counter. Without this, the next
     * mark of the session would be given one of them, and erasing either would
     * take both.
     */
    fun observeId(id: Long) {
        if (id >= nextId) nextId = id + 1
    }

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

    // ------------------------------------------------------------------ move --

    /**
     * Shift the mark with this [id] by [delta] page points.
     *
     * One edit for a whole drag, not one per moved pixel: the caller shows the
     * mark under the finger itself and calls this once, when the finger lifts.
     * Recording every frame would bury the rest of the history under a few
     * hundred one-point nudges.
     *
     * @return true when a mark was found and moved.
     */
    fun move(id: Long, delta: Offset): Boolean {
        if (delta == Offset.Zero) return false
        for (page in byPage.values) {
            val index = page.indexOfFirst { it.id == id }
            if (index < 0) continue
            val before = page[index]
            val after = before.movedBy(delta)
            // In place, so the mark keeps the depth it was drawn at: a moved
            // highlight that jumped to the top would come out over ink that had
            // been drawn on top of it.
            page[index] = after
            record(
                AnnotationEdit(
                    label = "move",
                    removed = listOf(PlacedAnnotation(index, before)),
                    added = listOf(after),
                ),
            )
            return true
        }
        return false
    }

    /**
     * Put a changed version of the text with this [id] in its place.
     *
     * One undo step per change, and the mark keeps both its id and its depth: it
     * is the same caption, restyled, not a new one dropped on top. [change] is
     * given the current version and returns the new one; returning the same thing
     * records nothing, so a control moved back to where it was does not leave an
     * undo step that does nothing.
     *
     * @return true when a caption was found and changed.
     */
    fun restyle(id: Long, label: String, change: (Annotation.Text) -> Annotation.Text): Boolean {
        for (page in byPage.values) {
            val index = page.indexOfFirst { it.id == id }
            if (index < 0) continue
            val before = page[index] as? Annotation.Text ?: return false
            val after = change(before)
            if (after == before) return false

            page[index] = after
            record(
                AnnotationEdit(
                    label = label,
                    removed = listOf(PlacedAnnotation(index, before)),
                    added = listOf(after),
                ),
            )
            return true
        }
        return false
    }

    /** The text with this [id], if it is one. */
    fun textMark(id: Long): Annotation.Text? =
        byPage.values.firstNotNullOfOrNull { page ->
            page.firstOrNull { it.id == id } as? Annotation.Text
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

    // ------------------------------------------------ following the page tree --

    /**
     * Renumber every mark after a page-tree edit, dropping those whose page went.
     *
     * **The annotation history is discarded.** Every [AnnotationEdit] records the
     * index a mark occupied on a page that may no longer exist, or may now hold
     * something else entirely; replaying one across a structural change would put
     * marks back onto whatever page happens to sit at that number now. Losing the
     * ability to undo a highlight is a far smaller harm than silently moving
     * somebody's annotations onto the wrong pages, and the document edit itself
     * stays undoable through the engine's own history either way.
     *
     * @return how many marks were dropped because their page was deleted.
     */
    fun remapPages(remap: PageRemap): Int {
        val surviving = mutableMapOf<Int, MutableList<Annotation>>()
        var dropped = 0

        // Ascending, so two pages merging into one keep their relative order
        // rather than depending on the map's iteration order.
        for (oldIndex in byPage.keys.sorted()) {
            val marks = byPage[oldIndex] ?: continue
            val newIndex = remap(oldIndex)
            if (newIndex == null) {
                dropped += marks.size
                continue
            }
            surviving.getOrPut(newIndex) { mutableListOf() }
                .addAll(marks.map { it.movedTo(newIndex) })
        }

        byPage.clear()
        byPage.putAll(surviving)
        done.clear()
        undone.clear()
        return dropped
    }

    /**
     * Turn every mark on [pageIndex] with its page.
     *
     * [width] and [height] are the page's size *before* the turn, since that is
     * the space the marks are currently expressed in.
     *
     * Clears the history for the same reason as [remapPages]: the stored edits
     * hold geometry in the old orientation.
     */
    fun rotatePage(pageIndex: Int, quarterTurns: Int, width: Float, height: Float) {
        val page = byPage[pageIndex] ?: return
        if (page.isEmpty()) return

        val turned = page.map { it.rotatedInPage(quarterTurns, width, height) }
        page.clear()
        page.addAll(turned)
        done.clear()
        undone.clear()
    }

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
        edit.added.forEach { annotation ->
            val page = pageOf(annotation.pageIndex)
            // A move records the same mark as both removed and added, and the
            // index it was removed from is the depth it is meant to keep. Only
            // marks that are genuinely new go on top.
            val index = edit.removed.firstOrNull { it.annotation.id == annotation.id }?.index
            if (index == null) page.add(annotation) else page.add(index.coerceIn(0, page.size), annotation)
        }
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

    /**
     * Picks out words. Drag across text and it snaps to the lines covered.
     *
     * Text only. Freehand used to be this tool's other mode, reached by a long
     * press that nobody would guess at; it is the pen now, in a slot of its own.
     */
    Highlight,

    // The drawing tools. One ribbon slot between them: the armed one is shown,
    // and a long press offers the rest.
    Pen,
    Line,
    Arrow,
    Rectangle,
    Ellipse,

    /**
     * A revision cloud: trace roughly around something and it is ringed in
     * scallops — the drawing-office way of saying "this part changed".
     *
     * Shares the circle's place in the palette rather than taking one of its own.
     * They answer the same question — how do I ring this — and the palette is
     * already five wide.
     */
    Cloud,

    /**
     * A curved line, and a curved arrow.
     *
     * Traced rather than dragged, and corrected: nobody draws a clean curve with
     * a finger, so the wobble is thrown away and replaced by the one smooth bend
     * it was meant to be. See `curveThrough`.
     */
    Curve,
    CurvedArrow,

    /**
     * Text written onto the page, straight or along a traced curve.
     *
     * Not a [Note], which is a marker with words behind it. This is text *on*
     * the page, and once saved it is real PDF text — selectable, searchable, and
     * part of the document rather than a mark laid over it.
     */
    Text,
    CurvedText,

    /**
     * Words with something drawn round them — a cloud, a box, an ellipse.
     *
     * One tool each rather than two marks made to line up by hand: the frame is
     * measured from the words, so choosing a point size sets its size too and
     * there is nothing to keep in step afterwards.
     */
    CloudText,
    BoxText,
    EllipseText,

    Note,
    Signature,

    /** Rubs out whole marks. Tap one, or sweep across several. */
    Eraser,
    Snapshot,
}

/**
 * The tools that draw, in the order the palette offers them.
 *
 * The pen first because it is the one most reached for, and the shapes after it
 * in the order a drawing needs them. The cloud is last and is not a slot of its
 * own — see [paletteTools].
 */
val DRAWING_TOOLS = listOf(
    AnnotationTool.Pen,
    AnnotationTool.Line,
    AnnotationTool.Arrow,
    AnnotationTool.Curve,
    AnnotationTool.CurvedArrow,
    AnnotationTool.Rectangle,
    AnnotationTool.Ellipse,
    AnnotationTool.Cloud,
    AnnotationTool.Text,
    AnnotationTool.CurvedText,
    AnnotationTool.CloudText,
    AnnotationTool.BoxText,
    AnnotationTool.EllipseText,
)

/**
 * Whether this tool writes words rather than drawing.
 *
 * It takes the same band as the drawing tools, but two of the slots mean
 * something else: the weight becomes a font size and the line type becomes the
 * font itself. Neither a nib width nor a dash means anything to a letter.
 */
val AnnotationTool.writesText: Boolean
    get() = this in TEXT_TOOLS

private val TEXT_TOOLS = setOf(
    AnnotationTool.Text,
    AnnotationTool.CurvedText,
    AnnotationTool.CloudText,
    AnnotationTool.BoxText,
    AnnotationTool.EllipseText,
)

/**
 * What this tool draws around the words it writes, if anything.
 *
 * The frame belongs to the tool rather than to a separate setting: picking
 * "clouded text" is picking the cloud, and there is no state in which a tool that
 * says it draws a box does not.
 */
/**
 * Whether this tool writes on a bent line.
 *
 * The bend is a setting rather than something drawn: a hand-drawn curve looked
 * broken however carefully it was traced, because a short caption covers only the
 * first part of a long stroke and the first part of any hand-drawn arc is its
 * straightest.
 */
val AnnotationTool.bendsText: Boolean get() = this == AnnotationTool.CurvedText

val AnnotationTool.textFrame: TextFrame
    get() = when (this) {
        AnnotationTool.CloudText -> TextFrame.Cloud
        AnnotationTool.BoxText -> TextFrame.Box
        AnnotationTool.EllipseText -> TextFrame.Ellipse
        else -> TextFrame.None
    }

/**
 * The drawing tools, in the slots the ribbon gives them.
 *
 * Grouped by what the mark *is*, not by how it is made: a line and an arrow are
 * one question, and so are the three ways of going round something. A slot shows
 * everything in its group at once with the armed one picked out, so the ribbon
 * says what is available as well as what is on — and a tap opens the group to
 * choose from rather than cycling, which reaches any member in one tap however
 * many the group grows to.
 *
 * The order inside a group is fixed. The row opens under a finger already moving
 * towards it, and if the armed one came first the same tap would land on a
 * different tool depending on what was armed when it opened.
 */
val DRAWING_GROUPS: List<List<AnnotationTool>> = listOf(
    listOf(
        AnnotationTool.Line,
        AnnotationTool.Arrow,
        AnnotationTool.Curve,
        AnnotationTool.CurvedArrow,
    ),
    listOf(AnnotationTool.Rectangle, AnnotationTool.Ellipse),
    listOf(AnnotationTool.Pen, AnnotationTool.Cloud),
    listOf(
        AnnotationTool.Text,
        AnnotationTool.CurvedText,
        AnnotationTool.CloudText,
        AnnotationTool.BoxText,
        AnnotationTool.EllipseText,
    ),
)

/** The tools sharing this one's slot, itself included, or just itself. */
val AnnotationTool.slotMates: List<AnnotationTool>
    get() = DRAWING_GROUPS.firstOrNull { this in it } ?: listOf(this)

/**
 * Whether this tool makes a mark, and so takes settings of its own.
 *
 * The highlighter is one: it has a colour, even though it has neither a nib
 * width nor a line type. One band serves them all — a second colour palette
 * behind a long press put the same colours on screen twice.
 */
val AnnotationTool.marks: Boolean get() = draws || this == AnnotationTool.Highlight

/** Whether this tool draws ink, and so takes a size, a colour and a line type. */
val AnnotationTool.draws: Boolean get() = this in DRAWING_TOOLS

/**
 * Whether this tool follows the finger rather than taking two corners.
 *
 * The pen keeps the path as drawn; the others replace it — the cloud with
 * scallops, the curves with the one smooth bend they were meant to be. Either way
 * the gesture is the same, trace the thing, which is why they share a branch in
 * the layer that captures it.
 */
val AnnotationTool.tracesPath: Boolean get() = this in TRACED_TOOLS

private val TRACED_TOOLS = setOf(
    AnnotationTool.Pen,
    AnnotationTool.Cloud,
    AnnotationTool.Curve,
    AnnotationTool.CurvedArrow,
)

/** Whether this tool builds its shape from two corners rather than tracing a path. */
val AnnotationTool.isDragged: Boolean get() = draws && !tracesPath


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

/**
 * What is drawn around a line of text.
 *
 * Sized from the type rather than dragged out: the point size decides the words
 * and the frame together, which is the whole reason these are one tool each
 * rather than a text mark and a shape that have to be lined up by hand.
 */
enum class TextFrame {
    None,

    /** The drawing-office revision cloud, scalloped outward. */
    Cloud,

    Box,

    /** An ellipse wide enough that the words sit inside it, not across it. */
    Ellipse,
}
