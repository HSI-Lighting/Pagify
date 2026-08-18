package com.hsilighting.pagify.core

import org.json.JSONArray
import org.json.JSONObject

/**
 * One edit to a document, as a value.
 *
 * Mirrors the `Command` enum in `rust/pdf_core/src/command/mod.rs`, including its
 * `{"op": ...}` tagging. The engine accepts nothing else: every mutation crosses
 * the boundary as one of these, which is what makes undo, batch processing and
 * saved scripts fall out of the design rather than needing to be retrofitted onto
 * each operation.
 *
 * The `op` strings and field names are a wire format shared with Rust. Renaming
 * one here without renaming it there produces a decode error at runtime, not a
 * compile error, so they are spelled out in one place and nowhere else.
 */
sealed interface PdfCommand {

    fun toJson(): String

    /**
     * Move pages. `order[i]` is the index the page currently at `i` moves to —
     * a destination map, not a "new order of old indices" list. The two agree on
     * the identity and on any simple swap, and disagree on everything else, so
     * getting it backwards looks right in casual testing.
     */
    data class ReorderPages(val order: List<Int>) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "reorderPages")
            put("order", JSONArray(order))
        }.toString()
    }

    data class DeletePage(val index: Int) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "deletePage")
            put("index", index)
        }.toString()
    }

    data class InsertBlankPage(
        val at: Int,
        val widthPoints: Float,
        val heightPoints: Float,
    ) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "insertBlankPage")
            put("at", at)
            put("widthPt", widthPoints.toDouble())
            put("heightPt", heightPoints.toDouble())
        }.toString()
    }

    /**
     * Rotation that is written into the file, unlike the view rotation the reader
     * applies at render time. [quarterTurns] is normalised into `0..3`, since a
     * UI that rotates repeatedly would otherwise send 4, 5, 6 and so on.
     */
    data class SetPageRotation(val index: Int, val quarterTurns: Int) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "setPageRotation")
            put("index", index)
            put("quarterTurns", ((quarterTurns % 4) + 4) % 4)
        }.toString()
    }
}

/**
 * The permutation that moves the page at [from] to position [to].
 *
 * A pure function and not a private helper in the view model, because it is the
 * one piece of this that is easy to get backwards and impossible to eyeball.
 * [PdfCommand.ReorderPages] wants a *destination map* — `order[i]` is where page
 * `i` ends up — while the natural way to compute a move is to build the resulting
 * *arrangement*, `arrangement[newIndex] = oldIndex`. Those two are inverses, and
 * they agree on the identity and on any single swap: the two cases anybody checks
 * by hand. They diverge on a move of three or more, which is every real drag.
 *
 * Returns the identity for a move that changes nothing or is out of range.
 */
fun reorderForMove(pageCount: Int, from: Int, to: Int): List<Int> {
    val identity = (0 until pageCount).toList()
    if (from == to || from !in identity.indices || to !in identity.indices) return identity

    val arrangement = identity.toMutableList()
    arrangement.add(to, arrangement.removeAt(from))

    val order = MutableList(pageCount) { 0 }
    arrangement.forEachIndexed { newIndex, oldIndex -> order[oldIndex] = newIndex }
    return order
}

/**
 * Everything the UI needs to redraw its edit controls, as of the last mutation.
 *
 * Returned by every edit call rather than assembled from separate queries: page
 * count, undo availability and dirtiness all change together, and reading them one
 * at a time would let the UI paint a state the document was never in.
 */
data class EditState(
    val pageCount: Int = 0,
    val canUndo: Boolean = false,
    val canRedo: Boolean = false,
    /** Already phrased as a user action, e.g. "Delete page 5". Null when empty. */
    val undoLabel: String? = null,
    val redoLabel: String? = null,
    /** Whether a save would have anything to write. */
    val dirty: Boolean = false,
    /** False for a document that cannot be edited; the UI hides its controls. */
    val editable: Boolean = false,
) {
    companion object {
        fun fromJson(json: String): EditState = JSONObject(json).run {
            EditState(
                pageCount = optInt("pageCount", 0),
                canUndo = optBoolean("canUndo", false),
                canRedo = optBoolean("canRedo", false),
                undoLabel = if (has("undoLabel") && !isNull("undoLabel")) {
                    getString("undoLabel")
                } else {
                    null
                },
                redoLabel = if (has("redoLabel") && !isNull("redoLabel")) {
                    getString("redoLabel")
                } else {
                    null
                },
                dirty = optBoolean("dirty", false),
                editable = optBoolean("editable", false),
            )
        }
    }
}
