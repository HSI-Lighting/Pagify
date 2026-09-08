package com.hsilighting.pagify.core

import org.json.JSONArray
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * A document the reader has opened before.
 *
 * Held by URI rather than by path because that is all the app is ever given: a
 * document arrives from a picker or another app as a `content://` URI, and the
 * grant that comes with it is what makes it openable again later. A path would be
 * unopenable on every device made this decade.
 *
 * The name, size and page count are copied rather than looked up when the list is
 * drawn. The library has to render instantly and offline, and asking a provider
 * about a dozen files — each a binder round trip, some of them cloud-backed — is
 * neither.
 */
data class RecentDocument(
    val uri: String,
    val name: String,
    /** Bytes, or 0 when the provider would not say. */
    val sizeBytes: Long,
    /** Pages, or 0 when the document failed before it was counted. */
    val pageCount: Int,
    val openedAtMillis: Long,
    /**
     * What kind of thing it is, and so which screen reopens it.
     *
     * Defaulted, because every entry written before this field existed is a
     * document and there is no version number in the file to tell them apart.
     * A missing kind meaning "document" is what lets an existing library
     * survive the upgrade instead of trying to open a PDF in the 3D viewer.
     */
    val kind: RecentKind = RecentKind.Document,
)

/** The screens a remembered file can be reopened in. */
enum class RecentKind(val stored: String) {
    Document("document"),
    Model("model"),
    Drawing("drawing"),
    ;

    companion object {
        /** Anything unrecognised is a document, including a missing value. */
        fun of(stored: String): RecentKind =
            entries.firstOrNull { it.stored == stored } ?: Document
    }
}

/**
 * The list after opening [document], newest first.
 *
 * Pure, and separate from the file it is stored in, because the ordering rules are
 * the part that can be quietly wrong: a document opened twice must move rather
 * than appear twice, and the list must stop growing at some point. Both are
 * invisible until the list is long, which is exactly when nobody is watching.
 *
 * Matching is by URI, so reopening the same file from a different picker session
 * still promotes the existing entry rather than adding a twin.
 */
fun promoteRecent(
    existing: List<RecentDocument>,
    document: RecentDocument,
    limit: Int = RECENT_DOCUMENT_LIMIT,
): List<RecentDocument> = (listOf(document) + existing.filterNot { it.uri == document.uri })
    .take(limit)

/**
 * How many documents the library remembers.
 *
 * Long enough to cover the file you were reading last week, short enough that the
 * list is still something you scan rather than search.
 */
const val RECENT_DOCUMENT_LIMIT = 40

/** Drop one entry — for a file that has been moved, deleted or revoked. */
fun forgetRecent(existing: List<RecentDocument>, uri: String): List<RecentDocument> =
    existing.filterNot { it.uri == uri }

/**
 * The documents whose names match what was typed.
 *
 * Case-insensitive and unanchored, so "nda" finds `2024-NDA-final.pdf`. A blank
 * query is not a filter — it returns everything rather than nothing, which is the
 * difference between a search box that is empty and one that has been cleared.
 */
fun searchRecents(documents: List<RecentDocument>, query: String): List<RecentDocument> {
    val needle = query.trim()
    if (needle.isEmpty()) return documents
    return documents.filter { it.name.contains(needle, ignoreCase = true) }
}

// ------------------------------------------------------------------ storage --

/**
 * The list as JSON, which is what gets written to disk.
 *
 * Hand-rolled with `org.json` rather than a serialisation library: this is one
 * flat object with five fields, and a dependency that pulls in a compiler plugin
 * to save fifteen lines is a poor trade.
 */
fun List<RecentDocument>.toRecentsJson(): String = JSONArray(
    map { document ->
        JSONObject().apply {
            put("uri", document.uri)
            put("name", document.name)
            put("sizeBytes", document.sizeBytes)
            put("pageCount", document.pageCount)
            put("openedAtMillis", document.openedAtMillis)
            put("kind", document.kind.stored)
        }
    },
).toString()

/**
 * Read the list back, skipping anything that will not parse.
 *
 * Lenient on purpose. This file is a convenience, not the user's data — the
 * documents themselves are untouched — so a half-written or older-format entry
 * should cost that one row, never the library screen.
 */
fun recentsFromJson(json: String): List<RecentDocument> {
    val array = runCatching { JSONArray(json) }.getOrNull() ?: return emptyList()
    val documents = mutableListOf<RecentDocument>()

    for (index in 0 until array.length()) {
        val entry = array.optJSONObject(index) ?: continue
        val uri = entry.optString("uri").takeIf { it.isNotBlank() } ?: continue
        documents += RecentDocument(
            uri = uri,
            name = entry.optString("name").ifBlank { uri.substringAfterLast('/') },
            sizeBytes = entry.optLong("sizeBytes", 0L),
            pageCount = entry.optInt("pageCount", 0),
            openedAtMillis = entry.optLong("openedAtMillis", 0L),
            kind = RecentKind.of(entry.optString("kind")),
        )
    }

    return documents
}

// ------------------------------------------------------------------ labels --

/**
 * A file size someone can read at a glance.
 *
 * Binary units, decimal only where it says something: "2.4 MB" is worth the
 * character, "2.4 KB" is noise next to a page count.
 */
fun formatFileSize(bytes: Long): String = when {
    bytes <= 0L -> ""
    bytes >= 1024L * 1024L -> "%.1f MB".format(bytes / (1024.0 * 1024.0))
    else -> "${(bytes + 1023L) / 1024L} KB"
}

/**
 * When a document was last opened, in the form the library shows.
 *
 * The date rather than "3 days ago": a list sorted by recency already says which
 * is newer, so the date is there to be recognised — "that is the one from the
 * October meeting" — and a relative label cannot do that.
 */
fun formatOpenedAt(millis: Long, locale: Locale = Locale.getDefault()): String {
    if (millis <= 0L) return ""
    return SimpleDateFormat("MMM d, yyyy", locale).format(Date(millis))
}

/** "12 pages · 2.4 MB", with either half dropped when it is not known. */
fun recentSubtitle(document: RecentDocument): String = listOf(
    formatOpenedAt(document.openedAtMillis),
    when {
        // A model has no pages, so the row says what it is instead. Without
        // this the only difference between a part and a document in the
        // library is that one of them shows no count — which reads as a
        // document that failed to open rather than as a 3D model.
        document.kind == RecentKind.Model -> "3D model"
        document.kind == RecentKind.Drawing -> "Drawing"
        document.pageCount > 0 ->
            "${document.pageCount} page${if (document.pageCount == 1) "" else "s"}"
        else -> ""
    },
    formatFileSize(document.sizeBytes),
).filter { it.isNotEmpty() }.joinToString("  ·  ")

/**
 * Which screen opens a file, from what it is called.
 *
 * **One picker, three viewers.** Asking somebody to say in advance whether they
 * are opening a PDF, a part or a drawing is asking them to answer a question
 * the file already answers — and getting it wrong means the picker they wanted
 * is the one they did not press. So there is one "Open a file" and this decides.
 *
 * By name, because that is all there is to go on: a `.dwg` arrives from most
 * providers as `application/octet-stream`, so filtering the picker by type
 * would hide the very files this exists to open.
 *
 * Anything unrecognised is a document. The PDF reader is the one that reports a
 * proper error for a file it cannot read, and its refusal is a sentence rather
 * than a shrug.
 */
fun kindOfFile(name: String): RecentKind = when (name.substringAfterLast('.', "").lowercase()) {
    "step", "stp", "p21" -> RecentKind.Model
    "dxf", "dwg" -> RecentKind.Drawing
    else -> RecentKind.Document
}

/** What "Open a file" will accept, for the line under it. */
const val SUPPORTED_FORMATS = "PDF, DWG, DXF, STEP"
