package com.hsilighting.pagify

import android.app.Application
import com.hsilighting.pagify.core.NativeBridge

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
        runCatching { NativeBridge.onTrimMemory(level) }
    }
}
