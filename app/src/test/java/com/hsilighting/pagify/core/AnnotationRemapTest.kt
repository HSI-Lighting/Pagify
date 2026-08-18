package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Marks have to follow their pages through a page-tree edit.
 *
 * This is the failure mode with no symptom: delete one page and every highlight
 * after it quietly slides onto the wrong page. Nothing on screen says so, and by
 * the time the user notices, the file has been saved.
 */
class AnnotationRemapTest {

    private fun store(vararg pages: Int): AnnotationStore = AnnotationStore().apply {
        pages.forEach { page ->
            add(
                Annotation.Highlight(
                    id = nextId(),
                    pageIndex = page,
                    // The width identifies the mark, so a test can tell which one
                    // ended up where without tracking ids.
                    rects = listOf(Rect(0f, 0f, page.toFloat() + 1f, 10f)),
                    color = AnnotationColors.YELLOW,
                ),
            )
        }
    }

    private fun widthOn(store: AnnotationStore, page: Int): Float? =
        (store.forPage(page).firstOrNull() as? Annotation.Highlight)?.rects?.first()?.right

    @Test
    fun `deleting a page pulls every later mark back with its page`() {
        val store = store(0, 1, 2, 3)

        val dropped = store.remapPages(PageRemap.afterDelete(1))

        assertEquals("the deleted page's own mark goes", 1, dropped)
        assertEquals(3, store.total)
        // Page 0 stays; old 2 and 3 become 1 and 2, carrying their own marks.
        assertEquals(1f, widthOn(store, 0))
        assertEquals(3f, widthOn(store, 1))
        assertEquals(4f, widthOn(store, 2))
    }

    @Test
    fun `inserting a page pushes later marks forward`() {
        val store = store(0, 1)

        assertEquals(0, store.remapPages(PageRemap.afterInsert(1)))

        assertEquals(1f, widthOn(store, 0))
        assertEquals("the mark that was on page 1 is now on page 2", 2f, widthOn(store, 2))
        assertTrue("the new blank page carries nothing", store.forPage(1).isEmpty())
    }

    @Test
    fun `a reorder sends each mark where its page went`() {
        val store = store(0, 1, 2)
        // Page 0 to the end: order[i] is where page i lands.
        val order = reorderForMove(3, from = 0, to = 2)

        store.remapPages(PageRemap.afterReorder(order))

        assertEquals(2f, widthOn(store, 0))
        assertEquals(3f, widthOn(store, 1))
        assertEquals("page 0's mark travelled to the end with it", 1f, widthOn(store, 2))
    }

    @Test
    fun `the annotation history is discarded, because its indices no longer mean anything`() {
        val store = store(0, 1)
        assertTrue(store.canUndo)

        store.remapPages(PageRemap.afterDelete(0))

        assertFalse(
            "replaying an edit recorded against the old numbering would put marks " +
                "back onto whatever page now holds that index",
            store.canUndo,
        )
        assertFalse(store.canRedo)
    }

    @Test
    fun `two pages merging into one keep both sets of marks`() {
        val store = store(0, 1)

        // A remap that is not injective — not something the page tree produces
        // today, but the store must not silently drop half the marks if one ever
        // does.
        store.remapPages { 0 }

        assertEquals(2, store.forPage(0).size)
    }

    // ------------------------------------------------------------- rotation --

    @Test
    fun `a quarter turn sends the top-left corner to the top-right`() {
        // A 100 x 200 page turned 90 degrees clockwise becomes 200 x 100, and the
        // corner that was at the origin ends up at the new page's right edge.
        val turned = Offset(0f, 0f).rotatedInPage(1, width = 100f, height = 200f)
        assertEquals(200f, turned.x)
        assertEquals(0f, turned.y)
    }

    @Test
    fun `four quarter turns are a full circle`() {
        val start = Offset(30f, 70f)
        var point = start
        // Width and height swap on every quarter turn, so they have to swap here
        // too — feeding the original page size to all four steps quietly gives the
        // wrong answer for a non-square page.
        var w = 100f
        var h = 200f
        repeat(4) {
            point = point.rotatedInPage(1, w, h)
            val swap = w
            w = h
            h = swap
        }
        assertEquals(start.x, point.x, 0.001f)
        assertEquals(start.y, point.y, 0.001f)
    }

    @Test
    fun `a rotated rect keeps its corners in order`() {
        val turned = Rect(10f, 20f, 40f, 60f).rotatedInPage(1, width = 100f, height = 200f)

        assertTrue("left must not end up past right", turned.left < turned.right)
        assertTrue("top must not end up past bottom", turned.top < turned.bottom)
        // A rect that inverts is stored happily and then fails every hit test,
        // which reads as a highlight that simply stopped working.
        assertEquals(40f, turned.width, 0.001f)
        assertEquals(30f, turned.height, 0.001f)
    }

    @Test
    fun `rotating a page turns the marks on it and leaves the others alone`() {
        val store = AnnotationStore()
        listOf(0, 1).forEach { page ->
            store.add(
                Annotation.Ink(
                    id = store.nextId(),
                    pageIndex = page,
                    points = listOf(Offset(0f, 0f)),
                    color = AnnotationColors.YELLOW,
                    strokeWidth = 2f,
                ),
            )
        }

        store.rotatePage(0, quarterTurns = 1, width = 100f, height = 200f)

        val moved = (store.forPage(0).first() as Annotation.Ink).points.first()
        val untouched = (store.forPage(1).first() as Annotation.Ink).points.first()
        assertEquals(Offset(200f, 0f), moved)
        assertEquals(Offset(0f, 0f), untouched)
    }
}
