package com.hsilighting.pagify.core

import android.content.Context
import android.content.pm.ApplicationInfo
import android.util.Log
import com.google.mlkit.vision.text.Text
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * What the recogniser actually returned, written out whole.
 *
 * ## Why this exists
 *
 * A card read on the device produced one field containing three separate things —
 * a title, a name and a company fused together. Two explanations fit, and they
 * lead to opposite fixes:
 *
 * * **ML Kit fused them**, in which case nothing on the Rust side can help and the
 *   answer is upstream, at a finer granularity.
 * * **Our own `into_lines` fused them**, re-grouping lines that arrived correctly
 *   separated — in which case the fix is to stop merging, not to merge more
 *   carefully.
 *
 * The app cannot tell the difference from its own output, because by the time
 * anything is visible both have already happened. So this writes down the
 * recogniser's view *before* we touch it.
 *
 * ## Why the whole hierarchy
 *
 * Blocks, lines **and** elements. The reduced shape the engine is sent — one box
 * per line — is exactly what would be unable to answer the question, and if the
 * answer turns out to be "work at element granularity" a reduced capture cannot
 * test it and every card would have to be photographed again.
 *
 * ## Debug builds only
 *
 * Guarded on whether the installed build is debuggable, which is the actual
 * question rather than a compile-time constant — and true only of the `.debug`
 * package that installs alongside the release one. This writes recognised text to
 * a file, which on a real card is somebody's name and telephone number; it has no
 * business running on anyone's phone but ours.
 */
object RecogniserDump {

    private const val TAG = "RecogniserDump"

    /** Whether a capture will be written at all. */
    fun enabled(context: Context): Boolean =
        (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    /**
     * Write the recogniser's full output beside the app's own files.
     *
     * External files, so it can be pulled with `adb pull` and no `run-as` — this
     * runs on a debug build installed next to the release one, which is not
     * debuggable.
     *
     * Never throws: a diagnostic that can break a scan is worse than no
     * diagnostic.
     */
    fun write(context: Context, text: Text, label: String) {
        if (!enabled(context)) return

        runCatching {
            val directory = File(context.getExternalFilesDir(null), "recogniser").apply {
                mkdirs()
            }
            val file = File(directory, "$label.json")
            file.writeText(asJson(text).toString(2))
            Log.i(TAG, "wrote ${file.absolutePath}")
        }.onFailure { Log.e(TAG, "the capture could not be written", it) }
    }

    /**
     * The hierarchy as JSON: blocks, their lines, and each line's elements.
     *
     * Corner points as well as the bounding box. A card photographed at an angle
     * has a rotated quadrilateral that a rectangle flattens, and "the box is
     * wider than the words" is one of the ways two columns come to overlap.
     */
    fun asJson(text: Text): JSONObject = JSONObject().apply {
        put("text", text.text)
        put(
            "blocks",
            JSONArray().apply {
                text.textBlocks.forEach { block ->
                    put(
                        JSONObject().apply {
                            put("text", block.text)
                            put("box", block.boundingBox.asJson())
                            put("corners", block.cornerPoints.asJson())
                            put("angle", block.lines.firstOrNull()?.angle ?: 0f)
                            put(
                                "lines",
                                JSONArray().apply {
                                    block.lines.forEach { line ->
                                        put(
                                            JSONObject().apply {
                                                put("text", line.text)
                                                put("box", line.boundingBox.asJson())
                                                put("corners", line.cornerPoints.asJson())
                                                put("angle", line.angle)
                                                put(
                                                    "elements",
                                                    JSONArray().apply {
                                                        line.elements.forEach { element ->
                                                            put(
                                                                JSONObject().apply {
                                                                    put("text", element.text)
                                                                    put(
                                                                        "box",
                                                                        element.boundingBox.asJson(),
                                                                    )
                                                                },
                                                            )
                                                        }
                                                    },
                                                )
                                            },
                                        )
                                    }
                                },
                            )
                        },
                    )
                }
            },
        )
    }
}

private fun android.graphics.Rect?.asJson(): JSONObject? = this?.let {
    JSONObject()
        .put("left", it.left)
        .put("top", it.top)
        .put("right", it.right)
        .put("bottom", it.bottom)
}

private fun Array<android.graphics.Point>?.asJson(): JSONArray = JSONArray().apply {
    this@asJson?.forEach { put(JSONObject().put("x", it.x).put("y", it.y)) }
}
