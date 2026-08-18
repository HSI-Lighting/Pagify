package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateCentroidSize
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.runtime.State
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import com.hsilighting.pagify.core.TwoFingerGesture
import androidx.compose.ui.unit.dp


/**
 * Two-finger zoom that leaves one-finger gestures alone.
 *
 * `detectTransformGestures` cannot be used here: it consumes single-pointer drags
 * as pan, which starves the surrounding scroll containers and makes the document
 * impossible to scroll. This handler only claims events once a *second* pointer is
 * down, so one finger still scrolls and two fingers zoom.
 *
 * [onZoomBy] receives the scale change **and the centroid** — the midpoint between
 * the fingers, in this element's local coordinates. The caller needs it because a
 * pinch must keep the content under that midpoint stationary; scaling about any
 * fixed point instead (the top-left corner, say) makes the page appear to slide
 * out from under the fingers.
 *
 * ## Why a two-finger gesture has to be classified before it is acted on
 *
 * Two fingers dragging in parallel never stay exactly parallel. The distance
 * between them wanders by a few pixels a frame, and every one of those wobbles
 * arrives as a zoom factor a hair away from 1.0. Acting on each one turned a
 * two-finger *scroll* into a slow zoom, which is what this gate exists to stop.
 *
 * Nothing is emitted until the gesture has declared itself: whichever of the two
 * measurements below first travels past [GESTURE_SLOP] decides what the gesture
 * is, and that decision holds until the fingers lift.
 *
 * - the fingers moving **apart or together** — a pinch
 * - the midpoint **travelling** — a pan
 *
 * Zoom is tested first, so a pinch anchored on one stationary finger — which
 * moves the midpoint about half as far as it changes the separation — still
 * reads as a zoom.
 *
 * @param onGestureEnd fired when the last finger lifts. A caller accumulating
 *   across a gesture needs this to reset, or what one gesture left behind is
 *   still sitting there when the next one starts.
 */
fun Modifier.pinchToZoom(
    onGestureEnd: () -> Unit = {},
    onZoomBy: (factor: Float, centroid: Offset) -> Unit,
): Modifier = composed {
    // `pointerInput` runs its block once for a constant key and captures
    // whatever it closed over at that moment. Routing the callback through
    // `rememberUpdatedState` is what keeps a later recomposition's lambda
    // visible to the already-running block — the exact trap that froze zoom
    // in the first implementation.
    val callback: State<(Float, Offset) -> Unit> = rememberUpdatedState(onZoomBy)
    val ended: State<() -> Unit> = rememberUpdatedState(onGestureEnd)

    pointerInput(Unit) {
        val slop = GESTURE_SLOP.toPx()

        awaitEachGesture {
            var pointerCount: Int
            var kind = TwoFingerGesture.Undecided
            /** Finger separation at the moment the second finger landed. */
            var startSpread = 0f
            var lastCentroid: Offset? = null
            /**
             * Total midpoint travel, summed frame by frame rather than measured
             * end to end, so a drag out and back still counts as movement.
             */
            var centroidTravel = 0f

            do {
                // Everything happens on the Initial pass, which travels parent
                // to child. A two-finger gesture must be claimed *before* any
                // nested scrollable sees it, or the content scrolls underneath
                // the fingers. Single-pointer events are left unconsumed on the
                // way past, so one-finger scrolling still works.
                val event = awaitPointerEvent(PointerEventPass.Initial)
                pointerCount = event.changes.count { it.pressed }

                if (pointerCount >= 2) {
                    val centroid = event.calculateCentroid(useCurrent = true)
                    val spread = event.calculateCentroidSize(useCurrent = true)

                    if (startSpread == 0f) {
                        startSpread = spread
                        lastCentroid = centroid
                    } else {
                        lastCentroid?.let { centroidTravel += (centroid - it).getDistance() }
                        lastCentroid = centroid

                        kind = TwoFingerGesture.classify(
                            current = kind,
                            spreadChange = spread - startSpread,
                            centroidTravel = centroidTravel,
                            slop = slop,
                        )

                        if (kind == TwoFingerGesture.Zoom) {
                            val zoom = event.calculateZoom()
                            if (zoom != 1f && zoom.isFinite() && zoom > 0f) {
                                callback.value(zoom, centroid)
                            }
                        }
                    }

                    // Claimed whichever way it went, so a scrollable underneath
                    // never fights a two-finger gesture for it. The panning half
                    // is delivered by `twoFingerPan`, reading the same events.
                    event.changes.forEach { it.consume() }
                }
            } while (pointerCount > 0)

            if (startSpread != 0f) ended.value()
        }
    }
}

/**
 * How far either measurement must travel before the gesture is called.
 *
 * Comfortably above the few pixels of noise two fingers produce dragging in
 * parallel, and well under the movement of a pinch anybody means.
 */
private val GESTURE_SLOP = 20.dp
