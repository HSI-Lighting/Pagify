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
        /**
         * What the sheet is made of, or null for a plain one.
         *
         * A PDF page has no background colour: white is only what an empty page
         * looks like. A coloured sheet is a filled rectangle covering it, so it
         * prints and it survives being opened anywhere else.
         */
        val fill: Long? = null,
        /**
         * What is printed on the sheet before anything is written on it: 0 plain,
         * then lined, squared, dotted. A sheet added to a notebook should match
         * the sheets already in it.
         */
        val ruling: Int = 0,
    ) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "insertBlankPage")
            put("at", at)
            put("widthPt", widthPoints.toDouble())
            put("heightPt", heightPoints.toDouble())
            fill?.let { put("fill", it.colorToWireJson()) }
            put("ruling", ruling)
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

    /**
     * Take words off a page, by the id the app gave the mark.
     *
     * By id rather than by position, because text is page content and page
     * content has no annotation index. The id is on every object the write put
     * there, so this finds all of them however the page has been edited since.
     */
    data class RemoveText(val pageIndex: Int, val id: Long) : PdfCommand {
        override fun toJson(): String = JSONObject().apply {
            put("op", "removeText")
            put("pageIndex", pageIndex)
            put("id", id.toInt())
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

    is Annotation.Shape -> JSONObject().apply {
        // Ink, like a signature: several strokes, which is what an arrow
        // needs and what a dashed anything comes out as. Every viewer draws
        // ink correctly, which a Square annotation cannot be relied on for.
        put("kind", "ink")
        put("strokes", JSONArray(strokes.map { it.toWireJson() }))
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

    is Annotation.Text -> textWireJson(withRestore = true)

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
 *
 * Shared with [Markup] rather than copied: two versions of a channel-order
 * conversion is two chances to get a shift wrong, and a wrong one is silent — a
 * red mark simply arrives blue.
 */
internal fun Long.colorToWireJson(): JSONObject = JSONObject().apply {
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

/**
 * A mark that is already in the document, with the index the engine addresses it
 * by.
 *
 * The index is **PDFium's**, not a position in any list: pages can hold form
 * widgets and links the engine does not model, and those are skipped on read. A
 * page holding a widget followed by a highlight yields one entry whose index is
 * 1, and erasing "the first mark" by list position would delete the widget.
 */
data class SavedMark(val index: Int, val annotation: Annotation)

/**
 * Parse the marks on a page as the engine reports them.
 *
 * The engine flattens the annotation's own fields alongside `index`, so this
 * reads one object rather than a nested pair — see `IndexedAnnotation` in
 * `rust/pdf_core/src/document/mod.rs`.
 */
fun savedMarksFromJson(json: String, pageIndex: Int, nextId: () -> Long): List<SavedMark> {
    val array = JSONArray(json)
    return buildList(array.length()) {
        for (i in 0 until array.length()) {
            val o = array.getJSONObject(i)
            val annotation = annotationFromWire(o, pageIndex, nextId()) ?: continue
            add(SavedMark(index = o.getInt("index"), annotation = annotation))
        }
    }
}

/** One mark from its wire form, or null for a shape this build cannot draw. */
private fun annotationFromWire(o: JSONObject, pageIndex: Int, id: Long): Annotation? {
    val color = o.optJSONObject("color").toColorLong()
    return when (o.optString("kind")) {
        "highlight" -> {
            val rects = o.optJSONArray("rects") ?: return null
            val parsed = (0 until rects.length()).map { rects.getJSONObject(it).toRect() }
            if (parsed.isEmpty()) null
            else Annotation.Highlight(id, pageIndex, parsed, color)
        }

        "ink" -> {
            val strokes = o.optJSONArray("strokes") ?: return null
            // The app's Ink is one stroke; the engine's is several, because a
            // signature is ink too. Several strokes come back as a Signature so
            // that nothing is silently dropped on the way in.
            val parsed = (0 until strokes.length()).map { s ->
                val points = strokes.getJSONArray(s)
                (0 until points.length()).map { points.getJSONObject(it).toOffset() }
            }.filter { it.size >= 2 }

            when {
                parsed.isEmpty() -> null
                parsed.size == 1 -> Annotation.Ink(
                    id = id,
                    pageIndex = pageIndex,
                    points = parsed[0],
                    color = color,
                    strokeWidth = o.optDouble("width", 2.0).toFloat(),
                )
                else -> Annotation.Signature(
                    id = id,
                    pageIndex = pageIndex,
                    strokes = parsed,
                    bounds = parsed.flatten().boundsOf(),
                    color = color,
                )
            }
        }

        "note" -> {
            val rect = o.optJSONObject("rect")?.toRect() ?: return null
            Annotation.Note(
                id = id,
                pageIndex = pageIndex,
                anchor = rect.center,
                text = o.optString("contents"),
                color = color,
            )
        }

        else -> null
    }
}

private fun JSONObject.toRect() = Rect(
    optDouble("left").toFloat(),
    optDouble("top").toFloat(),
    optDouble("right").toFloat(),
    optDouble("bottom").toFloat(),
)

private fun JSONObject.toOffset() = Offset(optDouble("x").toFloat(), optDouble("y").toFloat())

/** `0xAARRGGBB`, the form the app stores colours in. */
private fun JSONObject?.toColorLong(): Long {
    if (this == null) return AnnotationColors.YELLOW
    val a = optInt("a", 255).toLong() and 0xFF
    val r = optInt("r", 255).toLong() and 0xFF
    val g = optInt("g", 214).toLong() and 0xFF
    val b = optInt("b", 0).toLong() and 0xFF
    return (a shl 24) or (r shl 16) or (g shl 8) or b
}

private fun List<Offset>.boundsOf(): Rect {
    if (isEmpty()) return Rect(0f, 0f, 0f, 0f)
    return Rect(
        minOf { it.x },
        minOf { it.y },
        maxOf { it.x },
        maxOf { it.y },
    )
}

/**
 * A text mark on the wire.
 *
 * One object serves two readers, which is why it carries more than either needs.
 * The engine takes the glyphs, the ring and the colour and writes them; the app
 * takes the baseline, the frame kind and the packed colour and rebuilds the mark
 * from them when it reads the file again. Two objects would be two chances for
 * the same caption to come back as something slightly different.
 *
 * The glyphs are placed *here*, not by the engine, so the words land exactly
 * where the preview showed them. Both sides could walk the baseline, but only one
 * of them can be the authority on where a letter sits, and it has to be the side
 * the person was looking at when they put it there.
 */
internal fun Annotation.Text.textWireJson(withRestore: Boolean): JSONObject = JSONObject().apply {
    put("kind", "text")
    put("text", text)
    put("font", font.wireName)
    // The file to embed, when this font is one. Absent for a standard-14, which
    // is named rather than embedded and written by character.
    font.asset?.let { put("fontAsset", it) }
    put("size", sizePoints.toDouble())
    put("color", color.colorToWireJson())
    put(
        "glyphs",
        JSONArray(
            layOutBlock().map { glyph ->
                JSONObject().apply {
                    put("ch", glyph.text)
                    put("id", glyph.id)
                    put("x", glyph.origin.x.toDouble())
                    put("y", glyph.origin.y.toDouble())
                    put("radians", glyph.radians.toDouble())
                }
            },
        ),
    )
    put("id", id.toInt())
    // The ring goes with the words rather than beside them as its own mark.
    // Written separately it *was* separate once the file was reopened, and the
    // eraser took the ring off a clouded caption and left the words in place.
    put(
        "frame",
        JSONArray(
            textFrameOutline().map { point ->
                JSONObject().apply {
                    put("x", point.x.toDouble())
                    put("y", point.y.toDouble())
                }
            },
        ),
    )
    put("frameWidth", (sizePoints * TEXT_FRAME_STROKE).toDouble())

    // The app's half. The engine ignores fields it does not know, which is what
    // lets one object be both the instruction to write and the record of what
    // was written.
    put("argb", color)
    put("textFrame", frame.name)
    put(
        "path",
        JSONArray(
            path.map { point ->
                JSONObject().apply {
                    put("x", point.x.toDouble())
                    put("y", point.y.toDouble())
                }
            },
        ),
    )

    // Stored beside the words and handed back untouched. It is what makes a saved
    // caption a mark again rather than part of the page, and what lets erasing one
    // be undone — the engine reads it back as the annotation to write again.
    if (withRestore) put("restore", textWireJson(withRestore = false).toString())
}

/**
 * Rebuild a text mark from what was stored beside it.
 *
 * Returns null for anything it cannot read rather than a half-built mark: a
 * caption that comes back with the wrong words in the wrong place is worse than
 * one that stays part of the page.
 */
fun textMarkFromJson(json: String, pageIndex: Int): Annotation.Text? = try {
    val o = JSONObject(json)
    val points = o.getJSONArray("path")
    val path = (0 until points.length()).map { at ->
        val point = points.getJSONObject(at)
        Offset(point.getDouble("x").toFloat(), point.getDouble("y").toFloat())
    }
    if (path.isEmpty()) {
        null
    } else {
        Annotation.Text(
            id = o.getLong("id"),
            pageIndex = pageIndex,
            text = o.getString("text"),
            path = path,
            font = PdfFont.entries.firstOrNull { it.wireName == o.getString("font") }
                ?: PdfFont.HELVETICA,
            sizePoints = o.getDouble("size").toFloat(),
            color = o.getLong("argb"),
            frame = TextFrame.entries.firstOrNull { it.name == o.optString("textFrame") }
                ?: TextFrame.None,
        )
    }
} catch (_: org.json.JSONException) {
    null
}
