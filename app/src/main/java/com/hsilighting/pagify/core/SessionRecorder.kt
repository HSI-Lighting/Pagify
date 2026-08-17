package com.hsilighting.pagify.core

import java.io.File
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Records what the renderer actually did during a session, to a plain text file.
 *
 * The point is to replace guesswork with a timeline. Every thumbnail and every
 * page reports when it was asked for, when its outline was laid out, and when
 * each pass of pixels arrived — so a scroll that felt slow can be read back as
 * "these eleven thumbnails each took 240 ms and three were cache misses" rather
 * than described from memory.
 *
 * Written as text on purpose: the file is meant to be pulled off the device and
 * read, by a person or by a model, without any tooling.
 *
 * Call sites are cheap when not recording — an atomic read and an early return —
 * so the hooks can stay in release builds.
 */
object SessionRecorder {

    /** One recorded moment. Fields stay untyped; the file is for reading. */
    private data class Event(
        val atMillis: Long,
        val kind: String,
        val detail: String,
        val durationMillis: Long?,
    )

    private val recording = AtomicBoolean(false)
    private val events = ConcurrentLinkedQueue<Event>()

    @Volatile private var startedAtNanos = 0L
    @Volatile private var header: String = ""

    val isRecording: Boolean get() = recording.get()

    fun start(documentName: String, pageCount: Int, deviceNote: String) {
        events.clear()
        startedAtNanos = System.nanoTime()
        header = buildString {
            appendLine("Pagify session recording")
            appendLine("document : $documentName")
            appendLine("pages    : $pageCount")
            appendLine("device   : $deviceNote")
            appendLine("engine   : pdf_core ${NativeBridge.nativeVersion()}")
            appendLine()
            appendLine("t(ms)   event            detail")
            appendLine("------  ---------------  " + "-".repeat(60))
        }
        recording.set(true)
    }

    /**
     * @param durationMillis how long the work took, when the event marks a
     *   completion. Left null for instantaneous marks like an outline appearing.
     */
    fun record(kind: String, detail: String, durationMillis: Long? = null) {
        if (!recording.get()) return
        events += Event(
            atMillis = (System.nanoTime() - startedAtNanos) / 1_000_000,
            kind = kind,
            detail = detail,
            durationMillis = durationMillis,
        )
    }

    /**
     * Stop recording and write the timeline.
     *
     * @return the file written, or null if nothing was being recorded.
     */
    fun stop(directory: File): File? {
        if (!recording.getAndSet(false)) return null

        val snapshot = events.toList()
        events.clear()

        val file = File(directory, "pagify-session-${System.currentTimeMillis()}.txt")
        file.writeText(buildString {
            append(header)
            snapshot.forEach { event ->
                append(event.atMillis.toString().padStart(6))
                append("  ")
                append(event.kind.padEnd(15))
                append("  ")
                append(event.detail)
                event.durationMillis?.let { append("  took=${it}ms") }
                appendLine()
            }
            appendLine()
            append(summarise(snapshot))
        })
        return file
    }

    /**
     * A per-event-kind summary, so the interesting numbers do not have to be
     * eyeballed out of hundreds of lines. Median and worst case matter more than
     * the mean here: one 900 ms page is felt, a shifted average is not.
     */
    private fun summarise(snapshot: List<Event>): String = buildString {
        appendLine("Summary")
        appendLine("-".repeat(78))
        appendLine("total events : ${snapshot.size}")
        appendLine("duration     : ${snapshot.lastOrNull()?.atMillis ?: 0} ms")
        appendLine()
        appendLine("event            count   median    p95     max   (of timed events)")

        snapshot.groupBy { it.kind }.toSortedMap().forEach { (kind, group) ->
            val timings = group.mapNotNull { it.durationMillis }.sorted()
            append(kind.padEnd(15))
            append(group.size.toString().padStart(7))
            if (timings.isEmpty()) {
                appendLine("        -        -       -")
            } else {
                append(timings[timings.size / 2].toString().padStart(9))
                append(timings[(timings.size * 95 / 100).coerceAtMost(timings.lastIndex)]
                    .toString().padStart(8))
                append(timings.last().toString().padStart(8))
                appendLine()
            }
        }
    }
}
