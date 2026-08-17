package com.hsilighting.pagify.core

import org.json.JSONObject

/**
 * Document metadata, as produced by `DocumentMetadata` on the Rust side.
 *
 * Parsed with `org.json` rather than a serialisation library: it is part of the
 * platform, the payload is a handful of flat strings, and this runs once per
 * document open.
 */
data class PdfMetadata(
    val title: String? = null,
    val author: String? = null,
    val subject: String? = null,
    val keywords: String? = null,
    val creator: String? = null,
    val producer: String? = null,
    /** Raw PDF date string, e.g. `D:20240131120000+01'00'`. */
    val creationDate: String? = null,
    val modificationDate: String? = null,
    val pageCount: Int = 0,
) {
    /** What to show in a title bar when the document declares no title. */
    fun displayTitle(fallback: String): String = title?.takeIf { it.isNotBlank() } ?: fallback

    companion object {
        fun fromJson(json: String): PdfMetadata {
            val obj = JSONObject(json)
            // Absent keys are omitted by the Rust serialiser rather than sent as
            // null, so `optString` would silently yield "" — hence the has() check.
            fun str(key: String): String? =
                if (obj.has(key) && !obj.isNull(key)) obj.getString(key) else null

            return PdfMetadata(
                title = str("title"),
                author = str("author"),
                subject = str("subject"),
                keywords = str("keywords"),
                creator = str("creator"),
                producer = str("producer"),
                creationDate = str("creationDate"),
                modificationDate = str("modificationDate"),
                pageCount = obj.optInt("pageCount", 0),
            )
        }
    }
}

/** Native cache counters, for the debug overlay. */
data class PdfCacheStats(
    val hits: Long,
    val misses: Long,
    val entries: Int,
    val usedBytes: Long,
    val budgetBytes: Long,
) {
    val hitRate: Float
        get() = (hits + misses).let { total -> if (total == 0L) 0f else hits.toFloat() / total }

    companion object {
        fun fromJson(json: String): PdfCacheStats = JSONObject(json).run {
            PdfCacheStats(
                hits = optLong("hits"),
                misses = optLong("misses"),
                entries = optInt("entries"),
                usedBytes = optLong("usedBytes"),
                budgetBytes = optLong("budgetBytes"),
            )
        }
    }
}
