package com.hsilighting.pagify.core

import android.content.ComponentCallbacks2

/**
 * Every pool of decoded bitmaps the app holds, in one place.
 *
 * There were three, budgeted independently, with nothing summing them and
 * `onTrimMemory` reaching only the first:
 *
 * | pool | cap | trimmed |
 * |---|---|---|
 * | the engine's page cache (native) | 160 MB | yes |
 * | [ThumbnailCache] | 48 MB | no — `trim()` existed and was never called |
 * | the reader's recent-raster map | **none** | no |
 *
 * The uncapped one was the largest. It holds full-page rasters, and a page
 * measured 4465 × 3157 on the test tablet — about 54 MB at `ARGB_8888` — so four
 * of them reach roughly 215 MB, more than the two capped pools combined. It was
 * also the only one nobody was watching.
 *
 * This is a process-wide registry rather than a parameter passed around because
 * the trim signal arrives at the `Application`, which has no route to a
 * `ViewModel`'s private map. Pools register themselves; the callback reaches all
 * of them.
 */
object BitmapPools {

    interface Pool {
        /** Shown in the report; keep it short. */
        val poolName: String

        fun bytesHeld(): Int

        /**
         * Release under pressure. [level] is a `ComponentCallbacks2` constant, so
         * a pool can shed half at moderate pressure and everything at critical
         * rather than treating every warning the same.
         */
        fun trimTo(level: Int)
    }

    private val pools = mutableListOf<Pool>()

    @Synchronized
    fun register(pool: Pool) {
        if (pools.none { it === pool }) pools += pool
    }

    @Synchronized
    fun unregister(pool: Pool) {
        pools.removeAll { it === pool }
    }

    /** Total decoded bitmap memory held on the Java side, in bytes. */
    @Synchronized
    fun totalBytes(): Int = pools.sumOf { it.bytesHeld() }

    @Synchronized
    fun trim(level: Int) {
        pools.forEach { it.trimTo(level) }
    }

    /** One line per pool plus a total, for a session recording or a log. */
    @Synchronized
    fun report(): String {
        val lines = pools.joinToString(" ") { "${it.poolName}=${it.bytesHeld() / MB}MB" }
        return "$lines total=${totalBytes() / MB}MB"
    }

    /**
     * True once the system is asking for memory back rather than merely noting
     * that the app is in the background.
     *
     * `TRIM_MEMORY_RUNNING_MODERATE` and above are the running-process warnings;
     * the background levels mean the app is no longer visible, at which point
     * dropping everything is free.
     */
    fun isCritical(level: Int): Boolean =
        level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW

    private const val MB = 1024 * 1024
}
