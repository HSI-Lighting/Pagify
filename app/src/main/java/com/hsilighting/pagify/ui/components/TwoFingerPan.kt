package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.runtime.State
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import com.hsilighting.pagify.core.SessionRecorder

/**
 * Scrolling with two fingers, for when one finger is busy annotating.
 *
 * While a tool is selected the list's own scrolling is switched off, because a
 * one-finger drag has to mean "draw" and cannot also mean "scroll" — the two
 * race, and the scroll container wins as soon as the drag passes touch slop, so
 * a highlight would turn into a page scroll partway through.
 *
 * This restores panning on a second finger. It claims events on the Initial pass
 * so a nested scrollable never sees them, and only once two pointers are down, so
 * single-finger drags pass through untouched to the drawing layer.
 *
 * @param onPan vertical movement in pixels since the last event, sign matching
 *   the gesture: dragging down gives a positive delta.
 */
fun Modifier.twoFingerPan(onPan: (Float) -> Unit): Modifier = composed {
    val pan: State<(Float) -> Unit> = rememberUpdatedState(onPan)

    pointerInput(Unit) {
        awaitEachGesture {
            var lastCentroidY: Float? = null
            var pointerCount: Int

            do {
                val event = awaitPointerEvent(PointerEventPass.Initial)
                val pressed = event.changes.filter { it.pressed }
                pointerCount = pressed.size

                if (pointerCount >= 2) {
                    val centroidY = pressed.map { it.position.y }.average().toFloat()
                    lastCentroidY?.let { previous ->
                        val delta = centroidY - previous
                        if (delta != 0f) pan.value(delta)
                    }
                    lastCentroidY = centroidY
                    event.changes.forEach { it.consume() }
                } else {
                    // Dropping back to one finger restarts the reference point, so
                    // lifting one of two fingers does not register as a huge jump.
                    lastCentroidY = null
                }
            } while (pointerCount > 0)
        }
    }
}

/** Records that a gesture was routed to a tool rather than to scrolling. */
fun recordToolGesture(tool: String, detail: String) {
    SessionRecorder.record("TOOL_GESTURE", "tool=$tool $detail")
}
