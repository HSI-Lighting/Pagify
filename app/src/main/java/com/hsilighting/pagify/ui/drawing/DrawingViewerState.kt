package com.hsilighting.pagify.ui.drawing

import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import com.hsilighting.pagify.core.CaptureExport
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.DrawingBridge
import com.hsilighting.pagify.ui.model.Frame
import com.hsilighting.pagify.ui.model.captureFileName
import com.hsilighting.pagify.ui.model.captureSize
import com.hsilighting.pagify.ui.model.cutOut
import com.hsilighting.pagify.ui.model.PROXY_FRACTION
import com.hsilighting.pagify.ui.model.modelBitmap
import com.hsilighting.pagify.ui.model.regionInCapture
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import kotlin.math.roundToInt

/**
 * One open drawing, and everything the screen knows about it.
 *
 * Deliberately the same shape as `ModelViewerState`, including the two bitmaps
 * and the dropped-rather-than-queued redraw, because the reasons are the same:
 * reading a drawing takes long enough to matter and every frame after that is a
 * software rasterisation. What is missing is orbit — a sheet has no other side.
 *
 * The capture machinery is *shared* rather than copied: the sizing, the file
 * naming and the cut-out all come from the model viewer's, so a drawing and a
 * part produce the same kind of picture and one change fixes both.
 */
class DrawingViewerState(
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
    var layers by mutableStateOf<List<DrawingLayer>>(emptyList())
        private set

    private var handle: Long = DrawingBridge.NO_DRAWING
    private var width = 0
    private var height = 0
    private var full: Bitmap? = null
    private var proxy: Bitmap? = null
    private var gesturing = false
    private var drawing: Job? = null
    private var version = 0

    fun open(path: String) {
        scope.launch {
            val opened = withContext(Dispatchers.Default) {
                runCatching { DrawingBridge.openDrawing(path) }
            }
            opened
                .onSuccess { newHandle ->
                    handle = newHandle
                    describe()
                    loading = false
                    // Fitted before the first frame: a drawing's coordinates can
                    // be anywhere at all — a site plan in survey coordinates sits
                    // hundreds of thousands of units from the origin — so an
                    // unfitted first view is reliably empty.
                    if (width > 0 && height > 0) DrawingBridge.fitDrawing(handle, width, height)
                    redraw()
                }
                .onFailure { failure ->
                    error = failure.message ?: "This drawing could not be opened."
                    loading = false
                }
        }
    }

    private fun describe() {
        val summary = runCatching { JSONObject(DrawingBridge.drawingSummaryJson(handle)) }
            .getOrNull() ?: return

        val shapes = summary.optInt("shapes")
        val notShown = summary.optInt("notShown")
        val size = summary.optJSONObject("size")

        subtitle = buildString {
            if (size != null) {
                append("%,.0f × %,.0f".format(size.optDouble("x"), size.optDouble("y")))
                append(if (summary.optBoolean("unitsDeclared")) " units · " else " units · ")
            }
            append("%,d shapes".format(shapes))
            append(" · %,d layers".format(summary.optInt("layers")))
        }

        // One line, always visible: a drawing missing its text and dimensions
        // still looks like the drawing, which is exactly why it has to say so.
        if (notShown > 0) {
            val reasons = summary.optJSONArray("skipped")
            val named = (0 until (reasons?.length() ?: 0)).mapNotNull {
                reasons?.optJSONObject(it)?.let { entry ->
                    "%,d %s".format(entry.optInt("count"), entry.optString("what"))
                }
            }
            warning = "Not shown: " + named.joinToString(", ")
        }

        readLayers()
    }

    private fun readLayers() {
        val array = runCatching {
            org.json.JSONArray(DrawingBridge.drawingLayersJson(handle))
        }.getOrNull() ?: return

        layers = (0 until array.length()).mapNotNull { at ->
            array.optJSONObject(at)?.let {
                DrawingLayer(
                    at = at,
                    name = it.optString("name"),
                    visible = it.optBoolean("visible", true),
                )
            }
        }
    }

    /** Turn one layer off, or back on. */
    fun showLayer(at: Int, visible: Boolean) {
        if (handle == DrawingBridge.NO_DRAWING) return
        DrawingBridge.showDrawingLayer(handle, at, visible)
        layers = layers.map { if (it.at == at) it.copy(visible = visible) else it }
        redraw()
    }

    fun resize(newWidth: Int, newHeight: Int) {
        if (newWidth <= 0 || newHeight <= 0) return
        if (newWidth == width && newHeight == height) return

        val first = width == 0
        width = newWidth
        height = newHeight
        full = modelBitmap(newWidth, newHeight)
        proxy = modelBitmap(
            (newWidth * PROXY_FRACTION).roundToInt(),
            (newHeight * PROXY_FRACTION).roundToInt(),
        )
        // **Refitted whenever the view's shape changes, not only the first
        // time.** The fit depends on the aspect, so a sheet fitted in portrait
        // and then turned sideways would sit cropped until something moved it.
        if (handle != DrawingBridge.NO_DRAWING) {
            if (first) DrawingBridge.fitDrawing(handle, newWidth, newHeight)
        }
        redraw()
    }

    fun beginGesture() {
        gesturing = true
    }

    fun endGesture() {
        gesturing = false
        redraw()
    }

    fun pan(across: Float, down: Float) {
        if (handle == DrawingBridge.NO_DRAWING) return
        DrawingBridge.panDrawing(handle, across, down, width, height)
        redraw()
    }

    fun zoom(by: Float) {
        if (handle == DrawingBridge.NO_DRAWING) return
        DrawingBridge.zoomDrawing(handle, by)
        redraw()
    }

    fun fit() {
        if (handle == DrawingBridge.NO_DRAWING) return
        DrawingBridge.fitDrawing(handle, width, height)
        redraw()
    }

    /** Draw, unless a drawing is already in flight. See `ModelViewerState`. */
    private fun redraw() {
        if (handle == DrawingBridge.NO_DRAWING) return
        if (drawing?.isActive == true) return

        val target = (if (gesturing) proxy else full) ?: return
        drawing = scope.launch {
            val drawn = withContext(Dispatchers.Default) {
                runCatching { DrawingBridge.renderDrawingInto(handle, target) }.getOrElse {
                    Log.w(TAG, "the drawing could not be drawn", it)
                    false
                }
            }
            if (drawn) {
                version += 1
                picture = Frame(target, version)
            }
        }
    }

    fun close() {
        if (handle != DrawingBridge.NO_DRAWING) {
            DrawingBridge.closeDrawing(handle)
            handle = DrawingBridge.NO_DRAWING
        }
    }

    // ---- taking a picture ----------------------------------------------------

    var taken by mutableStateOf<Bitmap?>(null)
        private set
    var captureFormat by mutableStateOf(CaptureFormat.PNG)
        private set
    var capturing by mutableStateOf(false)
        private set
    var message by mutableStateOf<String?>(null)
        private set
    var shareRequest by mutableStateOf<Uri?>(null)
        private set

    fun chooseFormat(format: CaptureFormat) {
        captureFormat = format
    }

    fun messageShown() {
        message = null
    }

    fun shareRaised() {
        shareRequest = null
        discardCapture()
    }

    fun discardCapture() {
        taken = null
    }

    fun noteStorageRefused() {
        message = "Pagify needs permission to write to storage to save a picture."
    }

    /**
     * Take the region that was dragged.
     *
     * The whole sheet is drawn again at [scale] times the view and the region
     * cut from that, so a detail comes back with more in it than the screen
     * showed — the same promise the page and the model make, and the reason
     * this is worth more than a screenshot.
     */
    fun takeRegion(box: Rect, ring: List<Offset>, scale: Int = 2) {
        if (handle == DrawingBridge.NO_DRAWING || capturing) return
        val whole = captureSize(width, height, scale)
        val cut = regionInCapture(box, width, height, whole) ?: return

        capturing = true
        scope.launch {
            val drawn = withContext(Dispatchers.Default) {
                runCatching {
                    val target = modelBitmap(whole.width, whole.height)
                    if (!DrawingBridge.renderDrawingInto(handle, target)) return@runCatching null
                    cutOut(target, cut, ring, whole.width.toFloat() / width.toFloat())
                }.getOrElse {
                    Log.w(TAG, "the picture could not be drawn", it)
                    null
                }
            }
            capturing = false
            if (drawn != null) taken = drawn else message = "The picture could not be taken."
        }
    }

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
        picture.compress(kind, 92, out)
        return out.toByteArray()
    }

    private companion object {
        const val TAG = "DrawingViewer"
    }
}

/** One layer, and whether the sheet is showing it. */
data class DrawingLayer(val at: Int, val name: String, val visible: Boolean)
