package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.runtime.State
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput

/**
 * Two-finger zoom that leaves one-finger gestures alone.
 *
 * `detectTransformGestures` cannot be used here: it consumes single-pointer drags
 * as pan, which starves the surrounding scroll containers and makes the document
 * impossible to scroll. This handler only claims events once a *second* pointer is
 * down, so one finger still scrolls and two fingers zoom.
 */
fun Modifier.pinchToZoom(onZoomBy: (Float) -> Unit): Modifier = composed {
    // `pointerInput` runs its block once for a constant key and captures whatever
    // it closed over at that moment. Routing the callback through
    // `rememberUpdatedState` is what keeps a later recomposition's lambda visible
    // to the already-running block — the exact trap that froze zoom before.
    val callback: State<(Float) -> Unit> = rememberUpdatedState(onZoomBy)

    pointerInput(Unit) {
        awaitEachGesture {
            // Initial pass so a scroll container cannot claim the gesture first.
            awaitPointerEvent(PointerEventPass.Initial)

            var pointerCount: Int
            do {
                val event = awaitPointerEvent(PointerEventPass.Main)
                pointerCount = event.changes.count { it.pressed }

                if (pointerCount >= 2) {
                    val zoom = event.calculateZoom()
                    if (zoom != 1f && zoom.isFinite() && zoom > 0f) {
                        callback.value(zoom)
                    }
                    // Consumed only in the multi-touch case, so the scroll
                    // containers keep receiving single-finger drags untouched.
                    event.changes.forEach { it.consume() }
                }
            } while (pointerCount > 0)
        }
    }
}
