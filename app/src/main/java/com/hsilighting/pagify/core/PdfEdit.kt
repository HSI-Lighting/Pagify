package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
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

    /**
     * Put a mark into the document itself.
     *
     * Until this existed, annotations lived only in an [AnnotationStore] and were
     * gone the moment the document closed. They travel as a command like every
     * other mutation, which is why undo, redo, cache invalidation and the native
     * surface all work for them without any of it being written a second time.
     */
    data class AddAnnotation(val pageIndex: Int, val annotation: Annotation) : PdfCommand {
        // The mark is a *nested* object, not merged into the command. Merging
        // produced `{"op":"addAnnotation","kind":"highlight",...}`, which decodes
        // as `missing field 'annotation'` — the engine's variant holds an
        // `annotation` field and its own `kind` tag inside that.
        override fun toJson(): String = JSONObject().apply {
            put("op", "addAnnotation")
            put("pageIndex", pageIndex)
            put("annotation", annotation.toWireJson())
        }.toString()
    }

    /**
     * Take a mark out of the document. [index] is **PDFium's** index for it on the
     * page, not its position in any list the app holds.
     */
    data class RemoveAnnotation(val pageIndex: Int, val index: Int) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "removeAnnotation")
            put("pageIndex", pageIndex)
            put("index", index)
        }.toString()
    }
}

/**
 * This mark in the engine's wire form, without the command wrapper.
 *
 * Coordinates cross unchanged: both sides measure page points from the top-left
 * with y increasing downwards, and the engine flips to PDF's bottom-left
 * convention once, at the PDFium boundary. Flipping here as well would put every
 * saved mark on the wrong half of its page — and it would still look right in the
 * app, because the app would flip it back on the way in.
 */
fun Annotation.toWireJson(): JSONObject = when (this) {
    is Annotation.Highlight -> JSONObject().apply {
        put("kind", "highlight")
        put("rects", JSONArray(rects.map { it.toWireJson() }))
        put("color", color.colorToWireJson())
    }

    is Annotation.Ink -> JSONObject().apply {
        put("kind", "ink")
        // One stroke, but the engine models ink as several so a signature is the
        // same shape as a marker line rather than a special case.
        put("strokes", JSONArray(listOf(points.toWireJson())))
        put("color", color.colorToWireJson())
        put("width", strokeWidth.toDouble())
    }

    is Annotation.Signature -> JSONObject().apply {
        // Deliberately ink: a signature *is* a set of strokes, and giving it its
        // own annotation type would mean a second thing to read back, render and
        // erase for no difference anyone could see.
        put("kind", "ink")
        put("strokes", JSONArray(strokes.map { it.toWireJson() }))
        put("color", color.colorToWireJson())
        put("width", SIGNATURE_STROKE_WIDTH_POINTS.toDouble())
    }

    is Annotation.Note -> JSONObject().apply {
        put("kind", "note")
        put(
            "rect",
            JSONObject().apply {
                put("left", (anchor.x - NOTE_MARKER_RADIUS_POINTS).toDouble())
                put("top", (anchor.y - NOTE_MARKER_RADIUS_POINTS).toDouble())
                put("right", (anchor.x + NOTE_MARKER_RADIUS_POINTS).toDouble())
                put("bottom", (anchor.y + NOTE_MARKER_RADIUS_POINTS).toDouble())
            },
        )
        put("contents", text)
        put("color", color.colorToWireJson())
    }
}

private fun Rect.toWireJson(): JSONObject = JSONObject().apply {
    put("left", left.toDouble())
    put("top", top.toDouble())
    put("right", right.toDouble())
    put("bottom", bottom.toDouble())
}

private fun List<Offset>.toWireJson(): JSONArray = JSONArray(
    map { JSONObject().apply { put("x", it.x.toDouble()); put("y", it.y.toDouble()) } },
)

/**
 * A colour as the engine reads it.
 *
 * The app stores colours as `0xAARRGGBB` longs and the engine wants separate
 * bytes. Alpha is carried rather than dropped: a highlight is translucent, and
 * writing it opaque would black out the words underneath it.
 */
private fun Long.colorToWireJson(): JSONObject = JSONObject().apply {
    put("r", ((this@colorToWireJson shr 16) and 0xFF).toInt())
    put("g", ((this@colorToWireJson shr 8) and 0xFF).toInt())
    put("b", (this@colorToWireJson and 0xFF).toInt())
    put("a", ((this@colorToWireJson shr 24) and 0xFF).toInt())
}

/** Nib width written for a signature, in page points. */
const val SIGNATURE_STROKE_WIDTH_POINTS = 2f

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
