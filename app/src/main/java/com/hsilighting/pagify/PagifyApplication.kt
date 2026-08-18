package com.hsilighting.pagify

import android.app.Application
import com.hsilighting.pagify.core.BitmapPools
import com.hsilighting.pagify.core.NativeBridge
import com.hsilighting.pagify.core.SessionRecorder

class PagifyApplication : Application() {

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
