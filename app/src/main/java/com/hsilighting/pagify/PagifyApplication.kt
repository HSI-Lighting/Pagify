package com.hsilighting.pagify

import android.app.Application
import com.hsilighting.pagify.core.BitmapPools
import com.hsilighting.pagify.core.BundledFonts
import com.hsilighting.pagify.core.NativeBridge
import com.hsilighting.pagify.core.SessionRecorder

class PagifyApplication : Application() {

    /**
     * Give the engine its fonts before anything can ask for one.
     *
     * On a background thread: it is six megabytes of files read, parsed and
     * copied, and none of it is needed until somebody types. Registering is
     * idempotent, so a caption typed before it finishes finds the font a moment
     * later rather than losing it.
     */
    override fun onCreate() {
        super.onCreate()
        Thread { runCatching { BundledFonts.load(this) } }.start()
    }

    /**
     * Hands memory pressure straight to the native cache.
     *
     * The rasters the Rust core holds are invisible to the JVM heap accounting, so
     * without this the system's only lever against a document with a large cache
     * would be killing the process.
     */
    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        // Both sides. The native cache was the only one that ever heard this;
        // the Java-side pools are registered with `BitmapPools`, and together they
        // were the larger share.
        runCatching { NativeBridge.onTrimMemory(level) }
        runCatching {
            val before = BitmapPools.report()
            BitmapPools.trim(level)
            SessionRecorder.record(
                kind = "TRIM_MEMORY",
                detail = "level=$level before[$before] after[${BitmapPools.report()}]",
            )
        }
    }
}
