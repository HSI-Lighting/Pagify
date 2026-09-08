package com.hsilighting.pagify.core

import android.graphics.Bitmap

/**
 * The engine's drawing viewer, as Kotlin sees it.
 *
 * Mirrors [StepBridge] exactly in shape — a handle, ways to move the view, and
 * one call that fills a bitmap. What it does not have is `orbit`: a sheet has
 * no other side to turn to.
 */
internal object DrawingBridge {

    /** No drawing. The engine never hands back zero for a real one. */
    const val NO_DRAWING: Long = 0L

    init {
        // Referring to it loads it. The engine is one library; this is a third
        // door into it, not a third library.
        NativeBridge.nativeVersion()
    }

    /**
     * Open a `.dwg` or `.dxf` by path.
     *
     * Throws with the engine's own words when it cannot — a drawing that will
     * not open is common enough that the reason has to reach the screen.
     */
    external fun openDrawing(path: String): Long

    external fun closeDrawing(handle: Long): Boolean

    /** What the file holds, and what is not being shown, as JSON. */
    external fun drawingSummaryJson(handle: Long): String

    /** Every layer, with its colour and whether it is drawn. */
    external fun drawingLayersJson(handle: Long): String

    /**
     * Draw the sheet into a bitmap.
     *
     * [by] is how much larger this bitmap is than the one on screen — one for
     * an ordinary frame, two for a capture. A sheet's scale is pixels per
     * drawing unit, so without it a larger bitmap shows more of the drawing
     * instead of the same view in more detail.
     */
    external fun renderDrawingInto(handle: Long, bitmap: Bitmap, by: Float): Boolean

    /** Both distances are fractions of the view, not pixels. */
    external fun panDrawing(handle: Long, across: Float, down: Float, width: Int, height: Int): Boolean

    /**
     * Zoom about a point on the screen. Above one is closer.
     *
     * Takes where the fingers are: a pinch about the middle of the view has to
     * be dragged back afterwards every time.
     */
    external fun zoomDrawing(
        handle: Long,
        by: Float,
        atX: Float,
        atY: Float,
        width: Int,
        height: Int,
    ): Boolean

    /** Give the sheet a registered font to draw its text with. */
    external fun useDrawingFont(handle: Long, name: String): Boolean

    /**
     * Show the whole sheet.
     *
     * Needs the view's size, unlike the model viewer's fit: a camera can frame
     * a solid without knowing what shape of window it will be drawn into, and
     * a flat sheet cannot.
     */
    external fun fitDrawing(handle: Long, width: Int, height: Int): Boolean

    external fun showDrawingLayer(handle: Long, at: Int, visible: Boolean): Boolean

    /** For a test to prove that closing a drawing actually releases it. */
    external fun openDrawingCount(): Long
}
