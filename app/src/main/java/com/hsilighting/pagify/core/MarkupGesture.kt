package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset

/**
 * One drag on a capture, from touch-down to lift.
 *
 * A plain class rather than logic inside the pointer handler, because the rules
 * here are the ones worth testing and a Compose gesture cannot be driven from a
 * host test: when a stroke snaps, when it stays as drawn, and — the invariant the
 * work order names explicitly — that **nothing calls the engine while a finger is
 * down.** [up] is the only method that returns anything to act on.
 *
 * Everything is in page points; the caller converts before it gets here.
 */
class MarkupGesture(private val tool: MarkupTool) {

    private val points = mutableListOf<Offset>()

    /** True once the finger has held still long enough to mean it. */
    var isDwelling: Boolean = false
        private set

    /** What to draw while the finger is down, or null before it lands. */
    val preview: MarkupShape?
        get() = when {
            points.size < 2 -> null
            tool.isDragged -> tool.shapeFor(points.first(), points.last())
            else -> MarkupShape.Freehand(points.toList())
        }

    fun down(at: Offset) {
        points.clear()
        points += at
        isDwelling = false
    }

    fun move(to: Offset) {
        // Any real movement cancels a dwell: the hold has to be the *last* thing
        // that happened, or a pause halfway through a long squiggle would snap it.
        if (points.isEmpty() || (to - points.last()).getDistance() > MOVEMENT_SLOP) {
            isDwelling = false
        }
        points += to
    }

    /** The finger has been still for the dwell. Only meaningful for the pen. */
    fun still() {
        if (!tool.isDragged && points.size >= MINIMUM_STROKE_POINTS) isDwelling = true
    }

    /**
     * Lift, and what to do about it.
     *
     * The dwell decides whether recognition is even attempted. That is the whole
     * anti-surprise rule: a stroke drawn and lifted straight away is kept exactly
     * as drawn, and only a deliberate hold at the end asks for a shape. It also
     * keeps recognition out of the drag, where an engine call could cost a frame.
     */
    fun up(): Outcome {
        val stroke = points.toList()
        points.clear()
        val dwelled = isDwelling
        isDwelling = false

        if (stroke.size < 2) return Outcome.Nothing

        return when {
            tool.isDragged -> {
                val shape = tool.shapeFor(stroke.first(), stroke.last())
                if (shape.isBigEnough()) Outcome.Commit(shape) else Outcome.Nothing
            }
            // Held still at the end: ask the engine what this is.
            dwelled -> Outcome.Recognise(stroke)
            // Lifted straight away: keep it exactly as drawn.
            else -> Outcome.Commit(MarkupShape.Freehand(stroke))
        }
    }

    /** Abandoned — another pointer arrived, or the sheet closed. */
    fun cancel() {
        points.clear()
        isDwelling = false
    }

    sealed interface Outcome {
        /** Too small to be meant, or nothing was drawn. */
        data object Nothing : Outcome

        /** Ready to add, with no engine involved. */
        data class Commit(val shape: MarkupShape) : Outcome

        /**
         * Hand these points to the recogniser, then commit whatever comes back.
         *
         * Returned rather than recognised here so the engine call happens after
         * the finger is up, on the caller's terms.
         */
        data class Recognise(val points: List<Offset>) : Outcome
    }

    private companion object {
        /**
         * Movement below this does not cancel a dwell.
         *
         * A finger resting on glass still jitters by a point or so, and treating
         * that as movement would mean the dwell never completed on a real hand.
         */
        const val MOVEMENT_SLOP = 1.5f

        /** Too few points to be a shape; recognising three of them is guesswork. */
        const val MINIMUM_STROKE_POINTS = 6
    }
}

/** How long a finger must hold still, at the end of a stroke, to ask for a snap. */
const val MARKUP_DWELL_MILLIS = 300L

/**
 * Whether a dragged shape is worth keeping.
 *
 * Same reasoning as the capture rectangle: a tap that moved a couple of points is
 * someone trying to scroll, and turning it into an invisible mark that has to be
 * found and undone is worse than ignoring it.
 */
fun MarkupShape.isBigEnough(): Boolean = when (this) {
    is MarkupShape.Line -> (to - from).getDistance() >= MINIMUM_MARKUP_POINTS
    is MarkupShape.Arrow -> (to - from).getDistance() >= MINIMUM_MARKUP_POINTS
    is MarkupShape.Rectangle -> rect.width >= MINIMUM_MARKUP_POINTS ||
        rect.height >= MINIMUM_MARKUP_POINTS
    is MarkupShape.Ellipse -> rect.width >= MINIMUM_MARKUP_POINTS ||
        rect.height >= MINIMUM_MARKUP_POINTS
    is MarkupShape.Highlight -> rect.width >= MINIMUM_MARKUP_POINTS &&
        rect.height >= MINIMUM_MARKUP_POINTS
    is MarkupShape.Freehand -> points.size > 1
}

/** Smallest mark worth committing, in page points. */
const val MINIMUM_MARKUP_POINTS = 4f
