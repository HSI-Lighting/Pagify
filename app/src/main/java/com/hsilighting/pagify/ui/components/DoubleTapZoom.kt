package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.runtime.State
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp

/**
 * Double tap to zoom, watched rather than claimed.
 *
 * `detectTapGestures(onDoubleTap = …)` cannot be used for this. It waits for a
 * *down* that nothing else has consumed, and in the reader something else always
 * has: every page carries its own tap handler — the one that opens a note and
 * clears a text selection — and a child sees the Main pass before its parent. So
 * the page consumed each tap and the zoom handler on the reader was never given a
 * gesture to recognise. It was not a wiring mistake; the handler was correct and
 * simply starved, which is why it looked like the feature had never been built.
 *
 * This watches the **Initial** pass, which travels parent to child, and consumes
 * nothing at all. Nothing downstream can starve it, and nothing downstream loses
 * anything either: a page still opens its note on a single tap. A double tap on a
 * note therefore does both, which is the right trade — the alternative is claiming
 * the first tap before the page sees it, and that would break opening a note.
 *
 * A tap here is strict: one finger, no movement past touch slop, and the second
 * landing near the first within the platform's double-tap window. A second finger
 * at any point abandons the attempt, so a pinch is never mistaken for two taps.
 *
 * @param onDoubleTap the position of the second tap, in this element's own pixels.
 */
fun Modifier.doubleTapToZoom(onDoubleTap: (Offset) -> Unit): Modifier = composed {
    // The same trap `pinchToZoom` documents: `pointerInput` captures its lambda
    // once, so a callback that closes over anything recomposed has to be read
    // through a state holder or it goes stale on the first recomposition.
    val callback: State<(Offset) -> Unit> = rememberUpdatedState(onDoubleTap)

    pointerInput(Unit) {
        val slop = viewConfiguration.touchSlop
        val nearby = DOUBLE_TAP_DISTANCE.toPx()
        val window = viewConfiguration.doubleTapTimeoutMillis

        /** When the previous tap lifted, or 0 when there is no tap to pair with. */
        var previousUpMillis = 0L
        var previousPosition = Offset.Zero

        awaitEachGesture {
            val down = awaitFirstDown(
                requireUnconsumed = false,
                pass = PointerEventPass.Initial,
            )
            val start = down.position
            var wasATap = true
            var upMillis = 0L

            // Follow the whole gesture on the Initial pass. Reading it here rather
            // than through a helper is what keeps this a pure observer: the helpers
            // that wait for an up all consume it.
            do {
                val event = awaitPointerEvent(PointerEventPass.Initial)
                if (event.changes.size > 1) wasATap = false

                event.changes.firstOrNull { it.id == down.id }?.let { change ->
                    if ((change.position - start).getDistance() > slop) wasATap = false
                    if (!change.pressed) upMillis = change.uptimeMillis
                }
            } while (event.changes.any { it.pressed })

            when {
                !wasATap || upMillis == 0L -> {
                    // A drag, a pinch, or a gesture that never lifted cleanly. It
                    // also cancels any tap waiting for a partner, because a tap and
                    // a drag are not a double tap however close together they are.
                    previousUpMillis = 0L
                }

                previousUpMillis != 0L &&
                    upMillis - previousUpMillis <= window &&
                    (start - previousPosition).getDistance() <= nearby -> {
                    // Cleared first: three taps in a row are one double tap and one
                    // spare, not two overlapping pairs.
                    previousUpMillis = 0L
                    callback.value(start)
                }

                else -> {
                    previousUpMillis = upMillis
                    previousPosition = start
                }
            }
        }
    }
}

/**
 * How far apart two taps may land and still be one double tap.
 *
 * Compose exposes a touch slop but not a double-tap slop, and touch slop alone is
 * too tight: the second tap of a real double tap lands a few millimetres from the
 * first, which on a dense screen is well past the slop meant for deciding whether
 * a finger is dragging.
 */
private val DOUBLE_TAP_DISTANCE = 32.dp
