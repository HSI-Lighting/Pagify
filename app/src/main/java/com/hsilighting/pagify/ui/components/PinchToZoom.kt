package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.runtime.State
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput

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
 */
fun Modifier.pinchToZoom(onZoomBy: (factor: Float, centroid: Offset) -> Unit): Modifier =
    composed {
        // `pointerInput` runs its block once for a constant key and captures
        // whatever it closed over at that moment. Routing the callback through
        // `rememberUpdatedState` is what keeps a later recomposition's lambda
        // visible to the already-running block — the exact trap that froze zoom
        // in the first implementation.
        val callback: State<(Float, Offset) -> Unit> = rememberUpdatedState(onZoomBy)

        pointerInput(Unit) {
            awaitEachGesture {
                var pointerCount: Int
                do {
                    // Everything happens on the Initial pass, which travels parent
                    // to child. A pinch must be claimed *before* any nested
                    // scrollable sees it, or the content scrolls underneath the
                    // fingers while it scales. Single-pointer events are left
                    // unconsumed on the way past, so scrolling still works.
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    pointerCount = event.changes.count { it.pressed }

                    if (pointerCount >= 2) {
                        val zoom = event.calculateZoom()
                        if (zoom != 1f && zoom.isFinite() && zoom > 0f) {
                            callback.value(zoom, event.calculateCentroid())
                        }
                        event.changes.forEach { it.consume() }
                    }
                } while (pointerCount > 0)
            }
        }
    }
