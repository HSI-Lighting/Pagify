package com.hsilighting.pagify.ui.model

import android.graphics.Bitmap
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.hsilighting.pagify.core.StepBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import kotlin.math.roundToInt

/**
 * One open model, and everything the screen knows about it.
 *
 * # Nothing heavy on the main thread
 *
 * Opening a file parses it and builds a mesh — seconds for a large part — and
 * every frame after that is a software rasterisation. Both happen on
 * [Dispatchers.Default]; the main thread only ever swaps in a finished bitmap.
 * A viewer that blocks while it thinks is a viewer that gets killed by the
 * system for not responding.
 *
 * # Two bitmaps
 *
 * A small one for while a finger is down and a full-size one for when it lifts.
 * They are kept rather than allocated per frame: a 1080-wide ARGB bitmap is
 * four megabytes, and making one sixty times a second is how a smooth feature
 * turns into a stuttering one.
 */
class ModelViewerState(
    val name: String,
    private val scope: CoroutineScope,
) {
    var picture by mutableStateOf<Frame?>(null)
        private set
    var loading by mutableStateOf(true)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var subtitle by mutableStateOf<String?>(null)
        private set
    var warning by mutableStateOf<String?>(null)
        private set

    private var handle: Long = StepBridge.NO_MODEL
    private var width = 0
    private var height = 0
    private var full: Bitmap? = null
    private var proxy: Bitmap? = null
    private var gesturing = false
    private var drawing: Job? = null
    private var version = 0

    /** Open a file. Everything slow happens away from the main thread. */
    fun open(path: String) {
        scope.launch {
            val opened = withContext(Dispatchers.Default) {
                runCatching { StepBridge.openModel(path) }
            }

            opened
                .onSuccess { newHandle ->
                    handle = newHandle
                    describe()
                    loading = false
                    redraw()
                }
                .onFailure { failure ->
                    // The message is the refusal itself — "this model has 7,981
                    // faces" — written on the Rust side for exactly this.
                    error = failure.message ?: "This model could not be opened."
                    loading = false
                }
        }
    }

    private fun describe() {
        val summary = runCatching { JSONObject(StepBridge.modelSummaryJson(handle)) }.getOrNull()
            ?: return
        val size = summary.optJSONObject("size")
        val triangles = summary.optInt("triangles")

        subtitle = buildString {
            if (size != null) {
                append(
                    "%.0f × %.0f × %.0f mm".format(
                        size.optDouble("x"),
                        size.optDouble("y"),
                        size.optDouble("z"),
                    ),
                )
                append(" · ")
            }
            append("%,d triangles".format(triangles))
        }

        // **Said out loud, not logged.** A part drawn with faces missing looks
        // like the part; without this line there is nothing at all to tell
        // somebody that what they are looking at is incomplete.
        val skipped = summary.optJSONArray("skipped")
        if (skipped != null && skipped.length() > 0) {
            val total = (0 until skipped.length()).sumOf {
                skipped.optJSONObject(it)?.optInt("count") ?: 0
            }
            val reasons = (0 until skipped.length())
                .mapNotNull { skipped.optJSONObject(it)?.optString("what") }
                .distinct()
                .joinToString(", ")
            warning = "$total of ${summary.optInt("facesInFile")} faces are not shown ($reasons)."
        }
    }

    fun resize(newWidth: Int, newHeight: Int) {
        if (newWidth <= 0 || newHeight <= 0) return
        if (newWidth == width && newHeight == height) return

        width = newWidth
        height = newHeight
        full = modelBitmap(newWidth, newHeight)
        proxy = modelBitmap(
            (newWidth * PROXY_FRACTION).roundToInt(),
            (newHeight * PROXY_FRACTION).roundToInt(),
        )
        redraw()
    }

    fun beginGesture() {
        gesturing = true
    }

    /** The full-resolution picture arrives when the finger lifts. */
    fun endGesture() {
        gesturing = false
        redraw()
    }

    fun orbit(across: Float, down: Float) {
        if (handle == StepBridge.NO_MODEL) return
        StepBridge.orbitModel(handle, across, down)
        redraw()
    }

    fun pan(across: Float, down: Float) {
        if (handle == StepBridge.NO_MODEL) return
        StepBridge.panModel(handle, across, down)
        redraw()
    }

    fun zoom(by: Float) {
        if (handle == StepBridge.NO_MODEL) return
        StepBridge.zoomModel(handle, by)
        redraw()
    }

    fun fit() {
        if (handle == StepBridge.NO_MODEL) return
        StepBridge.fitModel(handle)
        redraw()
    }

    /**
     * Draw, unless a drawing is already in flight.
     *
     * **Dropped rather than queued.** A finger produces events faster than a
     * software rasteriser can answer them, and a queue would render every one
     * of them — arriving later and later behind the finger until the gesture
     * ends and a backlog plays out like a stutter. The newest camera position
     * is the only one worth drawing.
     */
    private fun redraw() {
        if (handle == StepBridge.NO_MODEL) return
        if (drawing?.isActive == true) return

        val target = (if (gesturing) proxy else full) ?: return
        drawing = scope.launch {
            val drawn = withContext(Dispatchers.Default) {
                runCatching { StepBridge.renderModelInto(handle, target) }.getOrElse {
                    Log.w(TAG, "the model could not be drawn", it)
                    false
                }
            }
            if (drawn) {
                // A new [Frame] rather than a copied bitmap: the same instance
                // with different pixels is, to Compose, the same value, so the
                // screen would never update -- but copying four megabytes a
                // frame to work around that costs more than the drawing did.
                version += 1
                picture = Frame(target, version)
            }
        }
    }

    /** Let the model go. The handle is native memory and does not collect itself. */
    fun close() {
        if (handle != StepBridge.NO_MODEL) {
            StepBridge.closeModel(handle)
            handle = StepBridge.NO_MODEL
        }
    }

    private companion object {
        const val TAG = "ModelViewer"
    }
}

/**
 * A drawn picture, and which drawing it was.
 *
 * The counter is the whole point. Compose compares by value, and a `Bitmap`
 * whose pixels changed is the same object — so without something that differs,
 * a redrawn model never reaches the screen.
 */
data class Frame(val bitmap: Bitmap, val version: Int)
