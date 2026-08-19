package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The wire format and the page arithmetic — the two places where a mistake is
 * silent rather than loud.
 *
 * A wrong `op` string decodes to an error at runtime and never at compile time;
 * a reversed permutation produces a reorder that is plausible and wrong.
 */
class PdfEditTest {

    // -------------------------------------------------------- the wire format --

    @Test
    fun `each command serialises to the tag the engine matches on`() {
        // These strings are shared with `Command` in rust/pdf_core/src/command/mod.rs.
        // Changing one here without changing it there fails only at runtime.
        assertEquals("deletePage", op(PdfCommand.DeletePage(3)))
        assertEquals("reorderPages", op(PdfCommand.ReorderPages(listOf(1, 0))))
        assertEquals("insertBlankPage", op(PdfCommand.InsertBlankPage(0, 595f, 842f)))
        assertEquals("setPageRotation", op(PdfCommand.SetPageRotation(2, 1)))
    }

    @Test
    fun `a delete carries the page it is deleting`() {
        val json = JSONObject(PdfCommand.DeletePage(7).toJson())
        assertEquals(7, json.getInt("index"))
    }

    @Test
    fun `an insert names its size in the fields the engine reads`() {
        val json = JSONObject(PdfCommand.InsertBlankPage(2, 595f, 842f).toJson())
        assertEquals(2, json.getInt("at"))
        // camelCase, matching serde's rename on the Rust side — `width_pt` would
        // decode as a missing field.
        assertEquals(595.0, json.getDouble("widthPt"), 0.001)
        assertEquals(842.0, json.getDouble("heightPt"), 0.001)
    }

    @Test
    fun `rotation is normalised into a quarter turn the engine accepts`() {
        // A UI that rotates repeatedly counts past 3, and negative turns come from
        // rotating the other way.
        assertEquals(1, turns(PdfCommand.SetPageRotation(0, 5)))
        assertEquals(0, turns(PdfCommand.SetPageRotation(0, 4)))
        assertEquals(3, turns(PdfCommand.SetPageRotation(0, -1)))
        assertEquals(2, turns(PdfCommand.SetPageRotation(0, -2)))
    }

    // --------------------------------------------------------------- the move --

    @Test
    fun `moving a page produces a destination map, not an arrangement`() {
        // Pages A B C D, moving A to the end. The arrangement is [1, 2, 3, 0] —
        // "the page now at 0 is the old page 1". The engine wants the inverse:
        // "the page at 0 moves to 3".
        assertEquals(listOf(3, 0, 1, 2), reorderForMove(4, from = 0, to = 3))
    }

    @Test
    fun `a move and its reverse cancel`() {
        val count = 5
        val forward = reorderForMove(count, from = 1, to = 4)
        val back = reorderForMove(count, from = 4, to = 1)

        // Applying one then the other must land every page where it started.
        val roundTrip = (0 until count).map { back[forward[it]] }
        assertEquals((0 until count).toList(), roundTrip)
    }

    @Test
    fun `every move is a genuine permutation`() {
        for (from in 0 until 6) {
            for (to in 0 until 6) {
                val order = reorderForMove(6, from, to)
                assertEquals(
                    "move $from -> $to must send each page to a distinct place",
                    (0 until 6).toSet(),
                    order.toSet(),
                )
            }
        }
    }

    @Test
    fun `a move that changes nothing is the identity`() {
        assertEquals(listOf(0, 1, 2), reorderForMove(3, from = 1, to = 1))
        assertEquals(listOf(0, 1, 2), reorderForMove(3, from = 9, to = 0))
        assertEquals(listOf(0, 1, 2), reorderForMove(3, from = 0, to = -1))
    }

    // ---------------------------------------------------------- the edit state --

    @Test
    fun `edit state decodes what the engine sends`() {
        val state = EditState.fromJson(
            """
            {"pageCount":4,"canUndo":true,"canRedo":false,
             "undoLabel":"Delete page 2","dirty":true,"editable":true}
            """.trimIndent(),
        )

        assertEquals(4, state.pageCount)
        assertTrue(state.canUndo)
        assertFalse(state.canRedo)
        assertEquals("Delete page 2", state.undoLabel)
        // Absent rather than null: serde omits `None` entirely, so a plain
        // optString would yield "" and the UI would label a button with nothing.
        assertNull(state.redoLabel)
        assertTrue(state.dirty)
        assertTrue(state.editable)
    }

    @Test
    fun `a document that cannot be edited decodes as not editable`() {
        val state = EditState.fromJson(
            """{"pageCount":9,"canUndo":false,"canRedo":false,"dirty":false,"editable":false}""",
        )

        assertEquals(9, state.pageCount)
        assertFalse(state.editable)
        assertFalse(state.dirty)
        assertNull(state.undoLabel)
    }

    // --------------------------------------------------------- annotations --

    @Test
    fun `a highlight command carries its rects and its colour`() {
        val json = JSONObject(
            PdfCommand.AddAnnotation(
                pageIndex = 4,
                annotation = Annotation.Highlight(
                    id = 1L,
                    pageIndex = 4,
                    rects = listOf(Rect(10f, 20f, 110f, 32f)),
                    color = 0x80FFD600L,
                ),
            ).toJson(),
        )

        assertEquals("addAnnotation", json.getString("op"))
        // Nested, not merged into the command. This assertion is the whole point
        // of the test: it previously read the flat shape the code happened to
        // produce, so it agreed with the bug instead of catching it, and the
        // engine rejected every mark with "missing field annotation".
        val mark = json.getJSONObject("annotation")
        assertEquals("highlight", mark.getString("kind"))
        // pageIndex is the field that would have slipped through camelCasing:
        // every other one in this command is a single word.
        assertEquals(4, json.getInt("pageIndex"))

        val rect = mark.getJSONArray("rects").getJSONObject(0)
        assertEquals(10.0, rect.getDouble("left"), 0.001)
        assertEquals(20.0, rect.getDouble("top"), 0.001)

        // Alpha survives. A highlight written opaque would black out the words
        // it is supposed to sit over.
        val colour = mark.getJSONObject("color")
        assertEquals(0x80, colour.getInt("a"))
        assertEquals(0xFF, colour.getInt("r"))
        assertEquals(0xD6, colour.getInt("g"))
        assertEquals(0x00, colour.getInt("b"))
    }

    @Test
    fun `a marker stroke becomes one ink stroke`() {
        val json = JSONObject(
            PdfCommand.AddAnnotation(
                pageIndex = 0,
                annotation = Annotation.Ink(
                    id = 2L,
                    pageIndex = 0,
                    points = listOf(Offset(1f, 2f), Offset(3f, 4f)),
                    color = 0xFF0000FFL,
                    strokeWidth = 3f,
                ),
            ).toJson(),
        )

        val mark = json.getJSONObject("annotation")
        assertEquals("ink", mark.getString("kind"))
        val strokes = mark.getJSONArray("strokes")
        assertEquals(1, strokes.length())
        assertEquals(2, strokes.getJSONArray(0).length())
        assertEquals(1.0, strokes.getJSONArray(0).getJSONObject(0).getDouble("x"), 0.001)
        assertEquals(3.0, mark.getDouble("width"), 0.001)
    }

    @Test
    fun `a signature is written as ink with all of its strokes`() {
        val json = JSONObject(
            PdfCommand.AddAnnotation(
                pageIndex = 2,
                annotation = Annotation.Signature(
                    id = 3L,
                    pageIndex = 2,
                    strokes = listOf(
                        listOf(Offset(0f, 0f), Offset(5f, 5f)),
                        listOf(Offset(6f, 0f), Offset(9f, 4f)),
                    ),
                    bounds = Rect(0f, 0f, 9f, 5f),
                    color = 0xFF000000L,
                ),
            ).toJson(),
        )

        // Ink, not a type of its own: a signature is strokes, and a separate kind
        // would be a second thing to read back, draw and erase for no visible gain.
        val mark = json.getJSONObject("annotation")
        assertEquals("ink", mark.getString("kind"))
        assertEquals(2, mark.getJSONArray("strokes").length())
    }

    @Test
    fun `a note becomes a rect around its anchor`() {
        val json = JSONObject(
            PdfCommand.AddAnnotation(
                pageIndex = 1,
                annotation = Annotation.Note(
                    id = 4L,
                    pageIndex = 1,
                    anchor = Offset(100f, 200f),
                    text = "check this",
                    color = 0xFFFFFF00L,
                ),
            ).toJson(),
        )

        val mark = json.getJSONObject("annotation")
        assertEquals("note", mark.getString("kind"))
        assertEquals("check this", mark.getString("contents"))
        val rect = mark.getJSONObject("rect")
        // Centred on the anchor: PDFium clips an annotation to its rect, so a
        // zero-area one would be written and then draw as nothing.
        assertEquals(100.0, (rect.getDouble("left") + rect.getDouble("right")) / 2, 0.001)
        assertEquals(200.0, (rect.getDouble("top") + rect.getDouble("bottom")) / 2, 0.001)
        assertTrue(rect.getDouble("right") > rect.getDouble("left"))
    }

    @Test
    fun `removing a mark addresses it by the engine's own index`() {
        val json = JSONObject(PdfCommand.RemoveAnnotation(pageIndex = 3, index = 7).toJson())

        assertEquals("removeAnnotation", json.getString("op"))
        assertEquals(3, json.getInt("pageIndex"))
        assertEquals(7, json.getInt("index"))
    }

    private fun op(command: PdfCommand): String = JSONObject(command.toJson()).getString("op")

    private fun turns(command: PdfCommand): Int =
        JSONObject(command.toJson()).getInt("quarterTurns")
}
