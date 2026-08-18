package com.hsilighting.pagify.core

import org.json.JSONArray

/**
 * A run of text on a page, and where it sits.
 *
 * Coordinates are in PDF points measured from the page's **top-left**, y
 * increasing downwards — the engine flips PDF's bottom-left convention once so
 * nothing above this has to remember to.
 *
 * These are page-space, not screen-space: multiply by the render scale to place
 * them over a drawn page, which keeps them valid at any zoom.
 *
 * A run is at most one line. **The order of a page's runs is meaningful** — the
 * engine returns them in the document's character order, which is the order the
 * page is read in — and [TextSelection] depends on it. Do not sort them.
 */
data class TextSegment(
    val left: Float,
    val top: Float,
    val right: Float,
    val bottom: Float,
    val text: String,
) {
    val width: Float get() = right - left
    val height: Float get() = bottom - top


    companion object {
        fun listFromJson(json: String): List<TextSegment> {
            val array = JSONArray(json)
            return buildList(array.length()) {
                for (i in 0 until array.length()) {
                    val o = array.getJSONObject(i)
                    add(
                        TextSegment(
                            left = o.getDouble("left").toFloat(),
                            top = o.getDouble("top").toFloat(),
                            right = o.getDouble("right").toFloat(),
                            bottom = o.getDouble("bottom").toFloat(),
                            text = o.optString("text"),
                        ),
                    )
                }
            }
        }
    }
}
