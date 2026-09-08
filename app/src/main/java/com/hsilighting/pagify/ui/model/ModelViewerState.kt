package com.hsilighting.pagify.ui.model

import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.StepBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.ByteArrayOutputStream
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

    /** Every row of the table, in the order it is read. */
    var details by mutableStateOf<List<DetailRow>>(emptyList())
        private set

    private fun describe() {
        val summary = runCatching { JSONObject(StepBridge.modelSummaryJson(handle)) }.getOrNull()
            ?: return
        val size = summary.optJSONObject("size")
        val drawn = summary.optInt("facesDrawn")
        val inFile = summary.optInt("facesInFile")

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
            append("%,d faces".format(inFile))
        }

        val surfaces = summary.optJSONObject("surfaces")
        val skipped = summary.optJSONArray("skipped")
        val lost = (0 until (skipped?.length() ?: 0)).sumOf {
            skipped?.optJSONObject(it)?.optInt("count") ?: 0
        }

        details = buildList {
            add(DetailRow.Heading("Model"))
            add(DetailRow.Item("Faces", "%,d".format(inFile)))
            // Only when they differ. "1,671 of 1,671" is a row to read and
            // discard; this line exists to say when something is missing.
            if (drawn != inFile) add(DetailRow.Item("Drawn", "%,d".format(drawn)))
            add(DetailRow.Item("Triangles", "%,d".format(summary.optInt("triangles"))))
            if (size != null) {
                add(
                    DetailRow.Item(
                        "Size",
                        "%.1f × %.1f × %.1f mm".format(
                            size.optDouble("x"),
                            size.optDouble("y"),
                            size.optDouble("z"),
                        ),
                    ),
                )
            }
            val assembly = summary.optInt("assembly")
            if (assembly > 0) add(DetailRow.Item("Assembly links", "%,d".format(assembly)))

            if (surfaces != null) {
                add(DetailRow.Heading("Surfaces"))
                // Named as somebody reading a drawing would name them, not as
                // STEP spells them: TOROIDAL_SURFACE means nothing to anybody
                // who has not read the standard.
                listOf(
                    "Flat" to "plane",
                    "Cylindrical" to "cylinder",
                    "Conical" to "cone",
                    "Toroidal" to "torus",
                    "Spherical" to "sphere",
                    "Freeform" to "freeform",
                ).forEach { (label, key) ->
                    val count = surfaces.optInt(key)
                    if (count > 0) add(DetailRow.Item(label, "%,d".format(count)))
                }
            }

            if (skipped != null && skipped.length() > 0) {
                add(DetailRow.Heading("Not shown"))
                for (index in 0 until skipped.length()) {
                    val entry = skipped.optJSONObject(index) ?: continue
                    add(
                        DetailRow.Item(
                            entry.optString("what").replaceFirstChar(Char::uppercase),
                            "%,d".format(entry.optInt("count")),
                        ),
                    )
                }
            }
        }

        // One line, always visible, because the table can be closed and a
        // part drawn with faces missing looks like the part.
        if (lost > 0) warning = "%,d of %,d faces are not shown.".format(lost, inFile)
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

    // ---- taking a picture ----------------------------------------------------

    /** The picture just taken, waiting to be saved, shared or copied. */
    var taken by mutableStateOf<Bitmap?>(null)
        private set

    /** Chosen on the sheet; PNG suits flat shading on a plain ground. */
    var captureFormat by mutableStateOf(CaptureFormat.PNG)
        private set

    var capturing by mutableStateOf(false)
        private set

    /** Something to tell the reader once, then forget. */
    var message by mutableStateOf<String?>(null)
        private set

    /** Set when a share sheet should be raised, once the bytes are on disk. */
    var shareRequest by mutableStateOf<Uri?>(null)
        private set

    fun chooseFormat(format: CaptureFormat) {
        captureFormat = format
    }

    fun messageShown() {
        message = null
    }

    /**
     * Say why nothing was saved.
     *
     * Naming the permission is the difference between somebody granting it and
     * concluding the feature is broken — the same reason the PDF reader says
     * it rather than failing quietly.
     */
    fun noteStorageRefused() {
        message = "Pagify needs permission to write to storage to save a picture."
    }

    fun shareRaised() {
        shareRequest = null
        discardCapture()
    }

    fun discardCapture() {
        taken = null
    }

    /**
     * Draw the model again, larger, and keep the result.
     *
     * Its own bitmap, never the one on screen: that one is redrawn by the next
     * gesture, and a picture that changes after it was taken is not a picture.
     */
    fun takePicture(scale: Int = 2) {
        if (handle == StepBridge.NO_MODEL || capturing) return
        val size = captureSize(width, height, scale)
        if (size.width <= 0 || size.height <= 0) return

        capturing = true
        scope.launch {
            val drawn = withContext(Dispatchers.Default) {
                runCatching {
                    val target = modelBitmap(size.width, size.height)
                    if (StepBridge.renderModelInto(handle, target)) target else null
                }.getOrElse {
                    Log.w(TAG, "the picture could not be drawn", it)
                    null
                }
            }
            capturing = false
            if (drawn != null) taken = drawn else message = "The picture could not be taken."
        }
    }

    /** Keep it, in Pictures/Pagify, where the gallery will find it. */
    fun savePicture(context: Context) =
        exportPicture(context, "Saved to Pictures/Pagify.") { bytes, fileName, format ->
            CaptureExport.saveToGallery(context, bytes, fileName, format)
            null
        }

    fun sharePicture(context: Context) =
        exportPicture(context, null) { bytes, fileName, _ ->
            CaptureExport.cache(context, bytes, fileName)
        }

    fun copyPicture(context: Context) =
        exportPicture(context, "Picture copied.") { bytes, fileName, _ ->
            CaptureExport.copyToClipboard(context, CaptureExport.cache(context, bytes, fileName))
            null
        }

    /**
     * Encode the picture and do something with the bytes.
     *
     * Both off the main thread. A six-megapixel PNG takes long enough that
     * encoding it inline stutters the sheet that is still on screen at the
     * time — and writing a file on the main thread is the other half of it.
     */
    private fun exportPicture(
        context: Context,
        note: String?,
        work: (ByteArray, String, CaptureFormat) -> Uri?,
    ) {
        val picture = taken ?: return
        val format = captureFormat
        val fileName = captureFileName(name, CaptureExport.timestamp(), format)

        scope.launch {
            val outcome = withContext(Dispatchers.IO) {
                runCatching { work(encode(picture, format), fileName, format) }
            }
            outcome
                .onSuccess { uri ->
                    if (uri != null) {
                        // The picture is kept until the share has actually been
                        // raised: dismissing first would take it out from under
                        // a chooser that has not appeared yet.
                        shareRequest = uri
                    } else {
                        message = note
                        discardCapture()
                    }
                }
                .onFailure { failure ->
                    Log.e(TAG, "the picture could not be exported", failure)
                    message = "The picture could not be saved."
                }
        }
    }

    private fun encode(picture: Bitmap, format: CaptureFormat): ByteArray {
        val out = ByteArrayOutputStream()
        val kind = when (format) {
            CaptureFormat.PNG -> Bitmap.CompressFormat.PNG
            CaptureFormat.JPEG -> Bitmap.CompressFormat.JPEG
        }
        // Ignored for PNG, which is lossless; 92 keeps a JPEG of flat shading
        // free of the ringing that shows up around a part's silhouette.
        picture.compress(kind, 92, out)
        return out.toByteArray()
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

/**
 * One line of the parameters table.
 *
 * Headings and values as separate kinds rather than a heading being an entry
 * with an empty value: a table read at a glance depends on the eye finding the
 * groups first, and that only works if they are drawn differently.
 */
sealed interface DetailRow {
    data class Heading(val text: String) : DetailRow
    data class Item(val label: String, val value: String) : DetailRow
}
