package com.hsilighting.pagify.core

import android.graphics.Bitmap
import android.util.LruCache

/**
 * In-memory cache of page thumbnails, held for the life of the open document.
 *
 * Thumbnails are small — around 100 KB each at 190 px wide — so an entire
 * document's worth fits comfortably in memory: roughly 10 MB for 95 pages, 15 MB
 * for 149. Scrolling the rail back over pages you have already seen should
 * therefore never re-render anything.
 *
 * It is deliberately separate from the engine's own cache. That one is budgeted
 * in bytes and shared with full-size page rasters, so a single 16 MB page render
 * could evict a hundred thumbnails at a stroke — which is exactly what made
 * scrolling up and down the rail feel like first-time work every time. Keeping
 * thumbnails in their own space means the two cannot compete.
 */
class ThumbnailCache(maxBytes: Int = DEFAULT_MAX_BYTES) : BitmapPools.Pool {

    private val entries = object : LruCache<Int, Bitmap>(maxBytes) {
        override fun sizeOf(key: Int, value: Bitmap): Int = value.byteCount
    }

    init {
        // Registered on construction, because `trim()` existed for a long time
        // with no caller at all: nothing routed the system's memory warning here.
        BitmapPools.register(this)
    }

    override val poolName = "thumbnails"

    override fun bytesHeld(): Int = entries.size()

    override fun trimTo(level: Int) {
        if (BitmapPools.isCritical(level)) clear() else trim()
    }

    fun get(pageIndex: Int): Bitmap? = entries.get(pageIndex)

    fun put(pageIndex: Int, bitmap: Bitmap) {
        entries.put(pageIndex, bitmap)
    }

    /** Called when the document changes; thumbnails of a closed file are useless. */
    fun clear() = entries.evictAll()

    /** Drop half the cache under memory pressure rather than all of it. */
    fun trim() = entries.trimToSize(entries.size() / 2)

    val usedBytes: Int get() = entries.size()
    val hitCount: Int get() = entries.hitCount()
    val missCount: Int get() = entries.missCount()

    private companion object {
        const val DEFAULT_MAX_BYTES = 48 * 1024 * 1024
    }
}
