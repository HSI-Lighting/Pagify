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
}
