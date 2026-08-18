package com.hsilighting.pagify.core

import kotlin.math.abs

/**
 * What a two-finger gesture turned out to be.
 *
 * Two fingers dragging in parallel never stay exactly parallel: the distance
 * between them wanders by a few pixels a frame, and each wobble reaches the pinch
 * handler as a zoom factor slightly off 1.0. Acting on them turned a two-finger
 * *scroll* into a slow zoom, so a gesture is classified before it is acted on and
 * the classification then holds until the fingers lift.
 *
 * Pure and separate from the pointer plumbing so the decision can be tested
 * against the numbers a real drag produces, rather than argued about.
 */
enum class TwoFingerGesture {
    /** Neither measurement has travelled far enough to say yet. */
    Undecided,
    Zoom,
    Pan,
    ;

    companion object {

        /**
         * Classify, or keep an existing decision.
         *
         * @param spreadChange how far the fingers are from the separation they
         *   started at. Signed movement is irrelevant — pinching in and out are
         *   both zooming — so the magnitude is what counts.
         * @param centroidTravel total distance the midpoint has travelled, summed
         *   frame by frame rather than measured end to end, so a drag out and
         *   back still counts as movement.
         * @param slop how far either has to travel to decide. Above the noise of
         *   a parallel drag, well below a deliberate pinch.
         */
        fun classify(
            current: TwoFingerGesture,
            spreadChange: Float,
            centroidTravel: Float,
            slop: Float,
        ): TwoFingerGesture {
            // A decision, once made, is kept: a pan that drifts apart near its end
            // must not turn into a zoom under the user's fingers.
            if (current != Undecided) return current
            return when {
                // Zoom is tested first so a pinch anchored on one stationary
                // finger still reads as a zoom. That gesture moves the midpoint
                // about half as far as it changes the separation, so on the frame
                // the separation passes the slop, the midpoint may have passed it
                // too — and the pinch is what was meant.
                abs(spreadChange) > slop -> Zoom
                centroidTravel > slop -> Pan
                else -> Undecided
            }
        }
    }
}

/**
 * Magnification reached so far in a pinch, before the magnified view exists.
 *
 * A plain running product, deliberately **unclamped**. Clamping each step at 1.0
 * made this a ratchet: the wobble of a two-finger scroll pushed it up and down in
 * equal measure, but only the upward half survived the clamp, so it crept towards
 * the handover threshold and eventually zoomed the reader in on its own — which
 * is why it only ever went *in*, never out. Any clamping belongs at the point the
 * value is used, not on the way in.
 */
fun pinchProgressAfter(progress: Float, factor: Float): Float = progress * factor
