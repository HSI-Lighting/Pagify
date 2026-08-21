package com.hsilighting.pagify.core

import org.json.JSONObject

/**
 * The settings that outlive a document.
 *
 * One object rather than a value per key, so reading and writing them is one
 * decision made once. The alternative — a getter and a setter per setting, each
 * remembering to write the file — is how a settings file ends up half-updated.
 */
data class AppSettings(
    val theme: ThemeChoice = ThemeChoice.SYSTEM,
    /**
     * Whether the viewfinder appears at all while zoomed.
     *
     * The hard off: no minimap and no handle to bring one back. [viewfinderMinimized]
     * is the soft one, for when it is wanted but not right now.
     */
    val showViewfinder: Boolean = true,
    /**
     * Whether the viewfinder is collapsed to its handle.
     *
     * Remembered across documents, because someone who finds it distracting finds
     * it distracting on the next drawing too — and the handle is right there when
     * they want it back.
     */
    val viewfinderMinimized: Boolean = false,
    /**
     * Where the folded handle sits, as fractions of the reader area.
     *
     * Fractions rather than pixels, so it stays where it was put when the phone
     * is turned or the rail appears — a handle remembered at 900 px down a
     * portrait screen is off the bottom of a landscape one.
     *
     * Right-hand edge, halfway down, to begin with: where the viewfinder itself
     * appears, so folding it away does not also move it.
     */
    val viewfinderHandleX: Float = 1f,
    val viewfinderHandleY: Float = 0.5f,
    /**
     * How the last capture was exported.
     *
     * Kept here rather than with the open document because it is a habit, not a
     * property of the file: somebody who sends screenshots into a report wants the
     * same sharpness and the same format every time, and being asked afresh on
     * every export is being asked a question already answered.
     */
    val captureScale: CaptureScale = CaptureScale.HIGH,
    val captureFormat: CaptureFormat = CaptureFormat.PNG,
    val captureFill: CaptureFill = CaptureFill.PAGE,
)

/** The settings as the file holds them. */
fun AppSettings.toSettingsJson(): String = JSONObject()
    .put(THEME_KEY, theme.name)
    .put(SHOW_VIEWFINDER_KEY, showViewfinder)
    .put(VIEWFINDER_MINIMIZED_KEY, viewfinderMinimized)
    .put(HANDLE_X_KEY, viewfinderHandleX.toDouble())
    .put(HANDLE_Y_KEY, viewfinderHandleY.toDouble())
    .put(CAPTURE_SCALE_KEY, captureScale.name)
    .put(CAPTURE_FORMAT_KEY, captureFormat.name)
    .put(CAPTURE_FILL_KEY, captureFill.name)
    .toString()

/**
 * Read them back, defaulting anything missing or unreadable.
 *
 * Lenient per key rather than all-or-nothing: a file written by an older build has
 * fewer keys than this one expects, and losing the theme because the viewfinder
 * setting did not exist yet would be a poor trade.
 */
fun settingsFromJson(json: String): AppSettings {
    val stored = runCatching { JSONObject(json) }.getOrNull() ?: return AppSettings()
    val defaults = AppSettings()

    return AppSettings(
        theme = themeChoiceFrom(stored.optString(THEME_KEY)),
        showViewfinder = stored.optBoolean(SHOW_VIEWFINDER_KEY, defaults.showViewfinder),
        viewfinderMinimized = stored.optBoolean(
            VIEWFINDER_MINIMIZED_KEY,
            defaults.viewfinderMinimized,
        ),
        viewfinderHandleX = stored
            .optDouble(HANDLE_X_KEY, defaults.viewfinderHandleX.toDouble())
            .toFloat()
            .coerceIn(0f, 1f),
        viewfinderHandleY = stored
            .optDouble(HANDLE_Y_KEY, defaults.viewfinderHandleY.toDouble())
            .toFloat()
            .coerceIn(0f, 1f),
        captureScale = stored.optString(CAPTURE_SCALE_KEY).toEnum(defaults.captureScale),
        captureFormat = stored.optString(CAPTURE_FORMAT_KEY).toEnum(defaults.captureFormat),
        captureFill = stored.optString(CAPTURE_FILL_KEY).toEnum(defaults.captureFill),
    )
}

/**
 * One stored name back to its value, or the default.
 *
 * By name rather than by ordinal: the scales were renumbered when they stopped
 * being 1x, 2x and 4x, and an ordinal written before that would have come back as
 * a different sharpness entirely.
 */
private inline fun <reified T : Enum<T>> String?.toEnum(fallback: T): T =
    enumValues<T>().firstOrNull { it.name == this } ?: fallback

private const val THEME_KEY = "theme"
private const val SHOW_VIEWFINDER_KEY = "showViewfinder"
private const val VIEWFINDER_MINIMIZED_KEY = "viewfinderMinimized"
private const val HANDLE_X_KEY = "viewfinderHandleX"
private const val HANDLE_Y_KEY = "viewfinderHandleY"
private const val CAPTURE_SCALE_KEY = "captureScale"
private const val CAPTURE_FORMAT_KEY = "captureFormat"
private const val CAPTURE_FILL_KEY = "captureFill"
