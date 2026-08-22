package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AnnotationStoreTest {

    private val store = AnnotationStore()

    private fun highlight(page: Int = 0, top: Float = 100f, left: Float = 10f) =
        Annotation.Highlight(
            id = store.nextId(),
            pageIndex = page,
            rects = listOf(Rect(left, top, left + 100f, top + 10f)),
            color = AnnotationColors.YELLOW,
        ).also { store.add(it) }

    private fun ink(page: Int = 0, from: Offset = Offset(0f, 0f), to: Offset = Offset(100f, 0f)) =
        Annotation.Ink(
            id = store.nextId(),
            pageIndex = page,
            points = listOf(from, to),
            color = AnnotationColors.RED,
            strokeWidth = 2f,
        ).also { store.add(it) }

    // ------------------------------------------------------------------ undo --

    @Test
    fun `nothing to undo on an untouched document`() {
        assertFalse(store.canUndo)
        assertFalse(store.canRedo)
        assertNull(store.undo())
    }

    @Test
    fun `undo removes the last mark and redo puts it back`() {
        highlight()
        val second = highlight(top = 200f)

        store.undo()
        assertEquals(listOf<Long>(1L), store.forPage(0).map { it.id })
        assertTrue(store.canRedo)

        store.redo()
        assertEquals(listOf(1L, second.id), store.forPage(0).map { it.id })
        assertFalse(store.canRedo)
    }

    @Test
    fun `a new mark abandons the redo branch`() {
        highlight()
        store.undo()
        assertTrue(store.canRedo)

        highlight(top = 300f)
        assertFalse("a fresh edit makes the undone branch unreachable", store.canRedo)
    }

    @Test
    fun `undo reports the page so the reader can go and show it`() {
        highlight(page = 7)
        assertEquals(7, store.undo()?.pageIndex)
    }

    // ---------------------------------------------------------------- eraser --

    @Test
    fun `erasing takes the mark under the point`() {
        val mark = highlight(top = 100f, left = 10f)
        highlight(top = 400f)

        assertTrue(store.eraseAt(0, Offset(50f, 105f), tolerance = 4f))
        assertFalse(store.forPage(0).any { it.id == mark.id })
        assertEquals(1, store.countOnPage(0))
    }

    @Test
    fun `erasing empty space takes nothing`() {
        highlight(top = 100f)
        assertFalse(store.eraseAt(0, Offset(50f, 700f), tolerance = 4f))
        assertEquals(1, store.countOnPage(0))
    }

    @Test
    fun `the topmost mark is the one erased`() {
        val under = highlight(top = 100f)
        val over = highlight(top = 100f)

        store.eraseAt(0, Offset(50f, 105f), tolerance = 4f)
        assertEquals(
            "the mark drawn last is the one under the finger",
            listOf(under.id),
            store.forPage(0).map { it.id },
        )
        assertFalse(store.forPage(0).any { it.id == over.id })
    }

    @Test
    fun `an ink stroke is erased by touching the line, not just its ends`() {
        ink(from = Offset(0f, 0f), to = Offset(100f, 100f))
        assertTrue(
            "a point on the diagonal must count as a hit",
            store.eraseAt(0, Offset(50f, 52f), tolerance = 4f),
        )
    }

    @Test
    fun `one sweep of the eraser is one undo`() {
        highlight(top = 100f)
        highlight(top = 200f)
        highlight(top = 300f)

        store.beginErase()
        store.eraseAt(0, Offset(50f, 105f), tolerance = 4f)
        store.eraseAt(0, Offset(50f, 205f), tolerance = 4f)
        store.eraseAt(0, Offset(50f, 305f), tolerance = 4f)
        store.endErase()

        assertEquals(0, store.countOnPage(0))
        store.undo()
        assertEquals("all three must come back together", 3, store.countOnPage(0))
    }

    /**
     * The bookkeeping that is easy to get wrong. Removing one mark shifts the
     * position of every mark after it, so an index recorded during a sweep is only
     * meaningful in the state that removal left behind — which is why undo has to
     * replay the removals backwards.
     */
    @Test
    fun `undoing a sweep restores the original stacking order`() {
        val ids = listOf(
            highlight(top = 100f),
            highlight(top = 200f),
            highlight(top = 300f),
            highlight(top = 400f),
            highlight(top = 500f),
        ).map { it.id }

        store.beginErase()
        // Deliberately out of order, and not adjacent: the second and the fourth.
        store.eraseAt(0, Offset(50f, 405f), tolerance = 4f)
        store.eraseAt(0, Offset(50f, 205f), tolerance = 4f)
        store.endErase()
        assertEquals(3, store.countOnPage(0))

        store.undo()
        assertEquals(
            "every mark must return to the position it held",
            ids,
            store.forPage(0).map { it.id },
        )
    }

    @Test
    fun `redoing a sweep takes the same marks away again`() {
        highlight(top = 100f)
        highlight(top = 200f)

        store.beginErase()
        store.eraseAt(0, Offset(50f, 105f), tolerance = 4f)
        store.eraseAt(0, Offset(50f, 205f), tolerance = 4f)
        store.endErase()

        store.undo()
        store.redo()
        assertEquals(0, store.countOnPage(0))

        // And the history is still coherent afterwards.
        store.undo()
        assertEquals(2, store.countOnPage(0))
    }

    // ----------------------------------------------------------------- clear --

    @Test
    fun `clearing a page leaves the other pages alone`() {
        highlight(page = 0)
        highlight(page = 0, top = 200f)
        highlight(page = 5)

        assertEquals(2, store.clearPage(0))
        assertEquals(0, store.countOnPage(0))
        assertEquals(1, store.countOnPage(5))
    }

    @Test
    fun `clearing a page is one undo`() {
        val ids = listOf(highlight(), highlight(top = 200f), highlight(top = 300f)).map { it.id }
        store.clearPage(0)

        store.undo()
        assertEquals(ids, store.forPage(0).map { it.id })
    }

    @Test
    fun `clearing everything is one undo across pages`() {
        highlight(page = 0)
        highlight(page = 3)
        highlight(page = 3, top = 200f)

        assertEquals(3, store.clearAll())
        assertTrue(store.isEmpty)

        store.undo()
        assertEquals(1, store.countOnPage(0))
        assertEquals(2, store.countOnPage(3))
    }

    @Test
    fun `clearing nothing records no history`() {
        assertEquals(0, store.clearAll())
        assertFalse("an empty clear is not an edit", store.canUndo)
    }

    // --------------------------------------------------------------- history --

    @Test
    fun `history is bounded but always leaves the recent edits undoable`() {
        repeat(260) { highlight(top = it * 2f) }
        var undone = 0
        while (store.undo() != null) undone++

        assertEquals("the history limit must hold", 200, undone)
        assertEquals("the marks beyond the limit stay put", 60, store.countOnPage(0))
    }

    @Test
    fun `closing the document forgets the history too`() {
        highlight()
        store.clear()
        assertFalse(store.canUndo)
        assertFalse(store.canRedo)
        assertTrue(store.isEmpty)
    }

    // ------------------------------------------------------------------ move --

    private fun text(page: Int = 0, at: Offset = Offset(50f, 200f)) =
        Annotation.Text(
            id = store.nextId(),
            pageIndex = page,
            text = "Approved",
            // A real baseline, as the view model builds one: a tap gives a point
            // and the layout walks a line, so a mark holding only the point is a
            // mark no glyph would ever land on.
            path = straightBaseline(at, "Approved", PdfFont.HELVETICA, 12f),
            font = PdfFont.HELVETICA,
            sizePoints = 12f,
            color = AnnotationColors.RED,
        ).also { store.add(it) }

    @Test
    fun `moving text takes it to the new place and keeps its id`() {
        val placed = text()
        assertTrue(store.move(placed.id, Offset(30f, -20f)))

        val moved = store.forPage(0).single() as Annotation.Text
        assertEquals(placed.id, moved.id)
        assertEquals(Offset(80f, 180f), moved.path.first())
        assertEquals("Approved", moved.text)
    }

    @Test
    fun `a move is one undo step back to where it started`() {
        val placed = text()
        store.move(placed.id, Offset(30f, -20f))

        store.undo()
        assertEquals(Offset(50f, 200f), (store.forPage(0).single() as Annotation.Text).path.first())
        // And only one step: the mark itself is still there, not undone away.
        assertEquals(1, store.forPage(0).size)

        store.redo()
        assertEquals(Offset(80f, 180f), (store.forPage(0).single() as Annotation.Text).path.first())
    }

    @Test
    fun `redoing a move leaves the mark at the depth it was drawn at`() {
        val under = text(at = Offset(10f, 10f))
        val over = ink()
        store.move(under.id, Offset(5f, 5f))

        store.undo()
        store.redo()
        // Ink was drawn on top and has to stay on top: a moved mark that came
        // back at the end of the list would be painted over it.
        assertEquals(listOf(under.id, over.id), store.forPage(0).map { it.id })
    }

    @Test
    fun `moving nothing changes nothing`() {
        val placed = text()
        assertFalse(store.move(placed.id, Offset.Zero))
        assertFalse(store.move(placed.id + 999L, Offset(10f, 10f)))
        assertFalse(store.canUndo && store.undo()?.label == "move")
    }

    @Test
    fun `every kind of mark moves`() {
        val marks = listOf(highlight(), ink())
        marks.forEach { store.move(it.id, Offset(7f, 11f)) }

        val movedHighlight = store.forPage(0).filterIsInstance<Annotation.Highlight>().single()
        assertEquals(Rect(17f, 111f, 117f, 121f), movedHighlight.rects.single())
        val movedInk = store.forPage(0).filterIsInstance<Annotation.Ink>().single()
        assertEquals(listOf(Offset(7f, 11f), Offset(107f, 11f)), movedInk.points)
    }

    @Test
    fun `restyling text rebuilds the line it sits on`() {
        val placed = text()
        assertTrue(store.restyle(placed.id, "size") { it.rebuilt(sizePoints = 24f) })

        val bigger = store.forPage(0).single() as Annotation.Text
        assertEquals(24f, bigger.sizePoints, 0.01f)
        // The layout walks the baseline and drops any glyph that runs off the
        // end, so type that grew on a line that did not would lose its last
        // letters. Every letter still lands.
        assertEquals(
            bigger.text.length,
            layOutText(bigger.text, bigger.font, bigger.sizePoints, bigger.path).size,
        )
        // And it grew from where it was placed, not from the page corner.
        assertEquals(placed.path.first(), bigger.path.first())
    }

    @Test
    fun `a new face gets a line long enough for it`() {
        val placed = text()
        store.restyle(placed.id, "font") { it.rebuilt(font = PdfFont.COURIER) }

        val reset = store.forPage(0).single() as Annotation.Text
        // Courier is wider than Helvetica at the same size. Keeping the old line
        // would drop the last letters off the end of it.
        assertEquals(
            reset.text.length,
            layOutText(reset.text, reset.font, reset.sizePoints, reset.path).size,
        )
    }

    @Test
    fun `a restyle is one undo step`() {
        val placed = text()
        store.restyle(placed.id, "size") { it.rebuilt(sizePoints = 24f) }
        store.undo()
        assertEquals(12f, (store.forPage(0).single() as Annotation.Text).sizePoints, 0.01f)
        assertEquals(1, store.forPage(0).size)
    }

    @Test
    fun `a control moved back to where it was records nothing`() {
        val placed = text()
        // Otherwise dragging a slider across and back leaves a pile of undo steps
        // that each do nothing, and the one real edit before them is unreachable.
        assertFalse(store.restyle(placed.id, "size") { it.rebuilt(sizePoints = 12f) })
        // Nothing recorded, so one undo still reaches past it to the placement
        // itself — which it could not do with a pile of no-op steps in the way.
        store.undo()
        assertTrue(store.forPage(0).isEmpty())
    }

    @Test
    fun `bending a caption keeps its words and where it starts`() {
        val placed = text()
        store.restyle(placed.id, "bend") { it.rebuilt(curveDegrees = 90f) }

        val bent = store.forPage(0).single() as Annotation.Text
        assertEquals(placed.text, bent.text)
        assertEquals(placed.path.first(), bent.path.first())
        assertEquals(90f, bent.curveDegrees, 0.01f)

        val placedGlyphs = layOutText(bent.text, bent.font, bent.sizePoints, bent.path)
        val turned = kotlin.math.abs(placedGlyphs.last().radians - placedGlyphs.first().radians)
        assertTrue("the line did not bend: $turned radians", turned > 1.0f)
    }

    @Test
    fun `restyling something that is not text changes nothing`() {
        val stroke = ink()
        assertFalse(store.restyle(stroke.id, "size") { it.rebuilt(sizePoints = 24f) })
    }

    @Test
    fun `straight text can be grabbed by its last letter, not only its first`() {
        val placed = text(at = Offset(50f, 200f))
        val end = 50f + PdfFont.HELVETICA.widthOf("Approved", 12f)
        // Just short of the end of the run, on the baseline: the point a reader
        // aiming at the final letter would touch.
        assertTrue(placed.isHitBy(Offset(end - 2f, 200f), tolerance = 1f))
        // And not far past it, or the whole line would be one big target.
        assertFalse(placed.isHitBy(Offset(end + 60f, 200f), tolerance = 1f))
    }
}
