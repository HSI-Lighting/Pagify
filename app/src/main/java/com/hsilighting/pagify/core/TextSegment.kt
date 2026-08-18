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

    /**
     * True when this run falls in the band swept between two y positions.
     *
     * Selection is by line, not by rectangle: dragging from the middle of one
     * line to the middle of another should take the whole of every line between,
     * including their left and right extremes, which is what a reader expects
     * and what a bounding-box intersection would not give.
     */
    fun intersectsBand(fromY: Float, toY: Float): Boolean {
        val top = minOf(fromY, toY)
        val bottom = maxOf(fromY, toY)
        return this.bottom >= top && this.top <= bottom
    }

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
