package com.hsilighting.pagify.core

import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.PixelCopy
import android.view.Window

/**
 * Watches for the reader going blank during a zoom, by reading actual screen
 * pixels.
 *
 * Deliberately measured from the composited window rather than from app state.
 * The blanking bugs found so far had causes the app could not see from the
 * inside — a render layer silently exceeding the GPU's maximum texture size, a
 * freshly composed view with no bitmap yet — and in both cases the app believed
 * it was drawing a page. Sampling the window is the only check that cannot be
 * fooled by that.
 *
 * Sampling begins the moment a zoom gesture touches the screen, runs every 50 ms
 * while the gesture is live and for a short tail afterwards, then stops. It is
 * inert unless a [SessionRecorder] session is running.
 *
 * ## Reading the results
 *
 * A blank *page* is legitimately white, so this reports the proportion of
 * non-white pixels rather than claiming a fault. `BLANK_START` means the content
 * area went below [NON_WHITE_THRESHOLD] non-white — for a text or image page
 * that means nothing is being drawn; for a genuinely near-empty page it is
 * expected. `BLANK_END` carries the duration, which is the number that matters.
 */
class BlankFrameDetector(private val window: Window) {

    private val thread = HandlerThread("pagify-blank-watch").apply { start() }
    private val handler = Handler(thread.looper)

    /** One capture, reused. The window is downscaled into this by `PixelCopy`. */
    private val sample = Bitmap.createBitmap(GRID, GRID, Bitmap.Config.ARGB_8888)
    private val pixels = IntArray(GRID * GRID)

    @Volatile private var lastActivityAtMillis = 0L
    @Volatile private var watching = false
    private var blankSinceMillis = 0L

    /**
     * The reader's own bounds in window coordinates, reported by the UI.
     *
     * Essential, not a refinement. Sampling the whole window meant the thumbnail
     * rail — around 15% of the width and full of colourful thumbnails — sat in
     * every sample, so the non-white fraction never fell near zero and a
     * completely blank page went undetected. The watcher has to look at exactly
     * the region that is supposed to be showing the page, and nothing else.
     */
    @Volatile private var contentBounds: Rect? = null

    fun setContentBounds(left: Int, top: Int, right: Int, bottom: Int) {
        if (right > left && bottom > top) {
            contentBounds = Rect(left, top, right, bottom)
        }
    }

    /** Call on every zoom gesture event; cheap and idempotent. */
    fun onZoomActivity() {
        lastActivityAtMillis = SystemClock.uptimeMillis()
        if (watching || !SessionRecorder.isRecording) return
        watching = true
        SessionRecorder.record("ZOOM_TOUCH", "watching for blank frames every ${INTERVAL_MS}ms")
        handler.post(::tick)
    }

    fun release() {
        watching = false
        handler.removeCallbacksAndMessages(null)
        thread.quitSafely()
    }

    private fun tick() {
        val idleFor = SystemClock.uptimeMillis() - lastActivityAtMillis
        if (!SessionRecorder.isRecording || idleFor > WATCH_TAIL_MS) {
            closeBlankRun()
            watching = false
            return
        }
        capture()
        handler.postDelayed(::tick, INTERVAL_MS)
    }

    private fun capture() {
        val decor = window.decorView
        if (decor.width <= 0 || decor.height <= 0) return

        // Prefer the reader's reported bounds; the inset is only a fallback for
        // before the first layout pass has run.
        val source = contentBounds ?: Rect(
            0,
            (decor.height * TOP_INSET_FRACTION).toInt(),
            decor.width,
            decor.height,
        )
        if (source.width() <= 0 || source.height() <= 0) return

        try {
            PixelCopy.request(window, source, sample, { result ->
                if (result == PixelCopy.SUCCESS) analyse()
            }, handler)
        } catch (t: IllegalArgumentException) {
            // The window has no surface yet, or is going away. Nothing to report.
        }
    }

    private fun analyse() {
        sample.getPixels(pixels, 0, GRID, 0, 0, GRID, GRID)

        var nonWhite = 0
        for (pixel in pixels) {
            val r = (pixel shr 16) and 0xFF
            val g = (pixel shr 8) and 0xFF
            val b = pixel and 0xFF
            if (r < WHITE_FLOOR || g < WHITE_FLOOR || b < WHITE_FLOOR) nonWhite++
        }

        val fraction = nonWhite.toFloat() / pixels.size
        val isBlank = fraction < NON_WHITE_THRESHOLD
        val now = SystemClock.uptimeMillis()

        if (isBlank && blankSinceMillis == 0L) {
            blankSinceMillis = now
            SessionRecorder.record(
                kind = "BLANK_START",
                detail = "nonWhite=${"%.2f".format(fraction * 100)}% grid=${GRID}x$GRID",
            )
        } else if (!isBlank && blankSinceMillis != 0L) {
            closeBlankRun()
        }
    }

    private fun closeBlankRun() {
        if (blankSinceMillis == 0L) return
        val duration = SystemClock.uptimeMillis() - blankSinceMillis
        blankSinceMillis = 0L
        SessionRecorder.record("BLANK_END", "screen recovered", duration)
    }

    private companion object {
        /** Sample resolution. 40k points is plenty and costs well under a millisecond to scan. */
        const val GRID = 200

        const val INTERVAL_MS = 50L

        /** Keep watching briefly after the last gesture: blanking often shows up on release. */
        const val WATCH_TAIL_MS = 3_000L

        /** Above this on all channels counts as white. */
        const val WHITE_FLOOR = 246

        /** Below this proportion of non-white pixels, the content area reads as blank. */
        const val NON_WHITE_THRESHOLD = 0.005f

        /** Skip the toolbar, which is a flat light surface. */
        const val TOP_INSET_FRACTION = 0.10f
    }
}
