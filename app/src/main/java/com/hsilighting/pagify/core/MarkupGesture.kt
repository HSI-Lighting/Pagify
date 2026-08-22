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
class MarkupGesture(
    private val tool: MarkupTool,
    /**
     * How heavy the tool is set, in page points.
     *
     * Only the cloud reads it, and only to size its scallops. It is a constructor
     * argument rather than something handed to [up] because the preview needs the
     * same number: a cloud previewed with one bump size and committed with
     * another would change shape as the finger left the glass.
     */
    private val sizePoints: Float = MARKUP_STROKE_POINTS,
) {

    private val points = mutableListOf<Offset>()

    /** True once the finger has held still long enough to mean it. */
    var isDwelling: Boolean = false
        private set

    /**
     * What to draw while the finger is down; empty before it lands.
     *
     * A list because one gesture is not always one shape: a curved arrow is the
     * curve and two barbs, kept apart so the tip stays sharp — a single polyline
     * running out to one barb and back would round off exactly where an arrow
     * needs its point.
     *
     * Built by the same call that will commit it, so what is under the finger is
     * what is released. Showing a raw trace and swapping it for scallops or a
     * clean arc on lift means aiming at something you cannot see.
     */
    val preview: List<MarkupShape>
        get() = when {
            points.size < 2 -> emptyList()
            tool.isDragged -> listOf(tool.shapeFor(points.first(), points.last()))
            // Shown as traced, corrected on lift. See `correctsOnRelease`:
            // re-deciding what a whole curve meant on every frame made the line
            // thrash about under the finger.
            tool.correctsOnRelease -> listOf(MarkupShape.Freehand(points.toList()))
            else -> tracedShapes(points)
        }

    /**
     * What a traced stroke becomes, for whichever tool traced it.
     *
     * The pen keeps its trace; the others replace it — the cloud with scallops,
     * the curves with the bends they were meant to be.
     */
    private fun tracedShapes(stroke: List<Offset>): List<MarkupShape> = when (tool) {
        MarkupTool.Cloud -> listOf(MarkupShape.Freehand(cloudOutline(stroke, sizePoints)))
        MarkupTool.Curve -> listOf(MarkupShape.Freehand(curveThrough(stroke)))
        MarkupTool.CurvedArrow ->
            curvedArrowStrokes(stroke, sizePoints).map { MarkupShape.Freehand(it) }
        else -> listOf(MarkupShape.Freehand(stroke.toList()))
    }.filter { it.isBigEnough() }

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
        if (tool.recognises && points.size >= MINIMUM_STROKE_POINTS) isDwelling = true
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
                if (shape.isBigEnough()) Outcome.Commit(listOf(shape)) else Outcome.Nothing
            }
            // Held still at the end: ask the engine what this is.
            dwelled -> Outcome.Recognise(stroke)
            // Traced. Nothing is asked of the engine — a cloud and a curve are
            // already the shapes they mean, and recognising one would hand back
            // the ellipse it was drawn around.
            else -> tracedShapes(stroke)
                .takeIf { it.isNotEmpty() }
                ?.let { Outcome.Commit(it) }
                ?: Outcome.Nothing
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

        /**
         * Ready to add, with no engine involved.
         *
         * Several shapes, because one gesture is not always one mark: a curved
         * arrow is a curve and two barbs, kept apart so its tip stays sharp.
         */
        data class Commit(val shapes: List<MarkupShape>) : Outcome

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
    // Text is placed and then typed; blank words make no mark rather than an
    // invisible one, which could only be found by rubbing out at random.
    is MarkupShape.Text -> text.isNotBlank() && path.isNotEmpty()
}

/** Smallest mark worth committing, in page points. */
const val MINIMUM_MARKUP_POINTS = 4f
