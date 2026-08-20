package com.hsilighting.pagify.data

import android.content.Context
import android.util.Log
import com.hsilighting.pagify.core.AppSettings
import com.hsilighting.pagify.core.settingsFromJson
import com.hsilighting.pagify.core.toSettingsJson
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.io.File

/**
 * The settings that outlive a document, kept across launches.
 *
 * One small JSON file, read once at startup and rewritten when something changes —
 * the same shape as [RecentDocumentsStore], and for the same reasons: a handful of
 * values does not need a database, and a file that fails to read should cost the
 * defaults rather than the launch.
 *
 * What the settings *are* lives in [AppSettings], which is pure and tested off
 * the device; this is only the file behind it.
 */
class AppSettingsStore(context: Context) {

    private val file = File(context.filesDir, FILE_NAME)

    private val _settings = MutableStateFlow(AppSettings())
    val settings: StateFlow<AppSettings> = _settings.asStateFlow()

    /**
     * Read the file.
     *
     * Until this completes the app draws with the defaults, which is the right
     * thing to show while the answer is unknown — and usually also the answer.
     */
    suspend fun load() {
        val stored = withContext(Dispatchers.IO) {
            runCatching {
                if (file.exists()) settingsFromJson(file.readText()) else AppSettings()
            }
                .onFailure { Log.w(TAG, "could not read the settings", it) }
                .getOrDefault(AppSettings())
        }
        _settings.value = stored
    }

    /**
     * Change one setting and write them all back.
     *
     * Takes the change as a transform rather than a value so a caller cannot
     * accidentally write a stale copy of everything else alongside its own edit.
     */
    suspend fun update(change: (AppSettings) -> AppSettings) {
        // In memory first: the screen should redraw on the tap, not on the write.
        val updated = change(_settings.value)
        if (updated == _settings.value) return
        _settings.value = updated

        withContext(Dispatchers.IO) {
            runCatching { file.writeText(updated.toSettingsJson()) }
                .onFailure { Log.w(TAG, "could not write the settings", it) }
        }
    }

    private companion object {
        const val FILE_NAME = "settings.json"
        const val TAG = "AppSettings"
    }
}
