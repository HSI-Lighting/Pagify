package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.sin

/**
 * A traced curve, corrected to the curve it was meant to be.
 *
 * Nobody draws a clean curve with a finger. The wobble is not information — it is
 * the hand — so the trace is thrown away and replaced by a smooth line through the
 * few points that actually say something: where it started, how deep each bend
 * went, where it changed direction, and where it ended.
 *
 * **Every bend is kept.** Changing direction part way through is how a hand says
 * "and then it goes the other way", so an S drawn as an S comes back as an S. The
 * hard part is telling that apart from shake, which reverses the turn at nearly
 * every touch sample: see [splitAtDirectionChanges].
 *
 * **One curve, not a chain of arcs.** Fitting each bend separately and joining
 * them puts a corner at every join — two arcs meeting arrive and leave at
 * different angles, and a curve with corners in it is not a curve. One spline
 * through all the guide points is smooth everywhere by construction, and it is
 * also what makes the line calm to draw: a new bend appends a guide point instead
 * of re-fitting everything before it.
 *
 * Sampled into a polyline rather than kept as a Bézier because ink has no curves —
 * a PDF ink annotation is a list of points, so the smoothness has to be in the
 * sampling.
 */
fun curveThrough(path: List<Offset>, segments: Int = CURVE_SEGMENTS): List<Offset> {
    if (path.size < 2) return emptyList()

    val from = path.first()
    val to = path.last()
    if ((to - from).getDistance() < MINIMUM_CURVE_POINTS) return emptyList()

    val guides = guidePoints(path)

    // Straight enough to have been meant straight. Bending it by a hair's breadth
    // would be correcting the hand in the wrong direction.
    if (guides.size < 3) return listOf(from, to)

    return splineThrough(guides, segments)
}

/**
 * A curve with a head on it, as the strokes to draw.
 *
 * The head is built from the curve's own last two points, so it points along the
 * direction the curve is actually travelling when it arrives — an arrow head
 * aligned to the straight line between the ends would sit visibly askew on
 * anything but a shallow bend.
 *
 * Three strokes rather than one, for the same reason a straight arrow is: a single
 * polyline running out to one barb and back would round off at the tip, and the
 * tip is the part an arrow is for.
 */
fun curvedArrowStrokes(
    path: List<Offset>,
    widthPoints: Float,
    segments: Int = CURVE_SEGMENTS,
): List<List<Offset>> {
    val curve = curveThrough(path, segments)
    if (curve.size < 2) return emptyList()

    val tip = curve.last()
    val approach = curve[curve.size - 2]
    val angle = atan2(tip.y - approach.y, tip.x - approach.x)
    val head = widthPoints * ARROW_HEAD_WIDTHS

    return listOf(curve) + listOf(-1f, 1f).map { side ->
        val barb = angle + Math.PI.toFloat() + side * ARROW_HEAD_ANGLE
        listOf(tip, Offset(tip.x + head * cos(barb), tip.y + head * sin(barb)))
    }
}

/**
 * What a traced stroke becomes, for whichever tool traced it.
 *
 * The counterpart of [shapeStrokes], which does the same job for the tools built
 * from two dragged corners. Here so that the preview and the commit cannot
 * disagree: both call this, so what is under the finger is what is released.
 *
 * The pen keeps its trace; the others replace it. Only the line along the ground
 * is dashed — an arrow's barbs are a few widths long, so a dash pattern lands on
 * one as a stub or misses it altogether.
 */
fun tracedStrokes(
    tool: AnnotationTool,
    path: List<Offset>,
    style: MarkupStyle,
    widthPoints: Float,
): List<List<Offset>> = when (tool) {
    AnnotationTool.Cloud -> dashed(cloudOutline(path, widthPoints), style, widthPoints)
    AnnotationTool.Curve -> dashed(curveThrough(path), style, widthPoints)
    AnnotationTool.CurvedArrow -> curvedArrowStrokes(path, widthPoints).let { strokes ->
        strokes.take(1).flatMap { dashed(it, style, widthPoints) } + strokes.drop(1)
    }
    else -> dashed(path, style, widthPoints)
}.filter { it.size > 1 }

/**
 * The handful of points that say what the trace meant.
 *
 * Where it began, the deepest point of each bend, where it changed direction, and
 * where it ended — nothing else. Every one of them comes from the evenly walked
 * trace rather than from the raw touch samples, so none of them carries shake.
 *
 * Fewer than three means the whole thing was straight, and the caller draws a
 * straight line rather than bending it by the width of a wobble.
 */
private fun guidePoints(path: List<Offset>): List<Offset> {
    val runs = splitAtDirectionChanges(path)
    val guides = mutableListOf(runs.first().first())

    runs.forEach { run ->
        // A run that barely bows has no apex worth steering through; its end point
        // alone carries it. A guide sitting on the line it guides costs nothing but
        // a chance to overshoot.
        deepestPoint(run)?.let { guides += it }
        guides += run.last()
    }

    return guides
}

/** The point of a run that strays furthest from its own chord, if any does. */
private fun deepestPoint(run: List<Offset>): Offset? {
    val from = run.first()
    val to = run.last()
    val length = (to - from).getDistance()
    if (length <= 0f) return null

    var deepest: Offset? = null
    var furthest = length * STRAIGHT_ENOUGH
    run.forEach { point ->
        val stray = abs(strayFromChord(point, from, to))
        if (stray > furthest) {
            furthest = stray
            deepest = point
        }
    }
    return deepest
}

/**
 * A smooth line through every one of [guides], in order.
 *
 * A Catmull-Rom spline: it passes *through* its control points rather than merely
 * near them, which matters because each was chosen to mean something — miss the
 * apex and the bend is shallower than the one that was drawn. It is also
 * continuous in direction at every control point, which is the whole reason for
 * running one spline through the lot instead of joining arcs end to end.
 *
 * The ends are doubled to give the first and last spans the neighbour the formula
 * needs. That makes the curve leave the start and arrive at the end heading
 * towards its neighbour, which is what a hand does.
 */
private fun splineThrough(guides: List<Offset>, segments: Int): List<Offset> {
    val padded = listOf(guides.first()) + guides + listOf(guides.last())
    val perSpan = (segments / (guides.size - 1)).coerceAtLeast(MINIMUM_SEGMENTS)

    val out = mutableListOf<Offset>()
    for (index in 1 until padded.size - 2) {
        val p0 = padded[index - 1]
        val p1 = padded[index]
        val p2 = padded[index + 1]
        val p3 = padded[index + 2]

        // Every span but the last stops one short of its end point: that point is
        // the next span's start, and drawing it twice would double a point.
        val last = if (index == padded.size - 3) perSpan else perSpan - 1
        for (step in 0..last) {
            out += catmullRom(p0, p1, p2, p3, step.toFloat() / perSpan)
        }
    }
    return out
}

/** One point along the Catmull-Rom span from [p1] to [p2]. */
private fun catmullRom(p0: Offset, p1: Offset, p2: Offset, p3: Offset, t: Float): Offset {
    val t2 = t * t
    val t3 = t2 * t
    return Offset(
        x = 0.5f * (
            2f * p1.x +
                (-p0.x + p2.x) * t +
                (2f * p0.x - 5f * p1.x + 4f * p2.x - p3.x) * t2 +
                (-p0.x + 3f * p1.x - 3f * p2.x + p3.x) * t3
            ),
        y = 0.5f * (
            2f * p1.y +
                (-p0.y + p2.y) * t +
                (2f * p0.y - 5f * p1.y + 4f * p2.y - p3.y) * t2 +
                (-p0.y + 3f * p1.y - 3f * p2.y + p3.y) * t3
            ),
    )
}

/**
 * The trace, cut where it changed which way it was bending.
 *
 * Deciding this on the raw points is hopeless: a hand shakes, so the direction of
 * the turn reverses at almost every touch sample and a plain arc would come back
 * as forty bends. Two things separate "the hand wobbled" from "the line turned" —
 * the trace is walked at even steps first, and the bend is measured as a bow
 * across a window rather than as a turn between two steps, so the threshold can be
 * a fraction of the stroke instead of an absolute number of points.
 *
 * A run also has to be long enough to be meant. A couple of steps leaning the
 * other way at the very end of a stroke is the finger lifting, not a second bend,
 * so short runs are folded into the one before them.
 */
private fun splitAtDirectionChanges(path: List<Offset>): List<List<Offset>> {
    val walked = evenlySpaced(path, TURN_SAMPLES)
    if (walked.size < 4) return listOf(walked)

    val length = walked.zipWithNext().sumOf { (a, b) -> (b - a).getDistance().toDouble() }
        .toFloat()
    val deadband = length * TURN_DEADBAND_FRACTION
    val window = (TURN_SAMPLES / 6).coerceAtLeast(2)

    // Which way the line is bending at each point: how far it bows from the chord
    // across a window either side of it, which is in the same units as the stroke.
    val turns = (window until walked.size - window).map { index ->
        val bow = strayFromChord(walked[index], walked[index - window], walked[index + window])
        when {
            bow > deadband -> 1
            bow < -deadband -> -1
            else -> 0
        }
    }

    val minimumRun = (TURN_SAMPLES * MINIMUM_RUN_FRACTION).toInt().coerceAtLeast(3)
    val cuts = mutableListOf<Int>()
    var side = 0
    var runLength = 0

    turns.forEachIndexed { index, turn ->
        when {
            turn == 0 -> runLength++
            side == 0 -> {
                side = turn
                runLength = 1
            }
            turn == side -> runLength++
            runLength >= minimumRun -> {
                // A real change of direction, and the run before it was long enough
                // to have been meant. Cut at the point of inflection.
                cuts += index + window
                side = turn
                runLength = 1
            }
        }
    }

    if (cuts.isEmpty()) return listOf(walked)

    val bounds = listOf(0) + cuts + listOf(walked.size - 1)
    return bounds.zipWithNext { from, to -> walked.subList(from, to + 1).toList() }
        .filter { it.size >= 2 }
}

/**
 * [count] points spread evenly along the path by length.
 *
 * By length rather than by index, because a hand slows at the corners: spacing by
 * index would spend most of the samples where the finger dawdled and almost none
 * along the fast part of the stroke, which is exactly where a change of direction
 * is easiest to miss.
 */
private fun evenlySpaced(path: List<Offset>, count: Int): List<Offset> {
    val total = path.zipWithNext().sumOf { (a, b) -> (b - a).getDistance().toDouble() }.toFloat()
    if (total <= 0f) return path

    val step = total / (count - 1)
    val out = ArrayList<Offset>(count)
    out += path.first()

    var travelled = 0f
    var target = step

    for ((from, to) in path.zipWithNext()) {
        val segment = (to - from).getDistance()
        if (segment <= 0f) continue
        while (travelled + segment >= target && out.size < count - 1) {
            out += from + (to - from) * ((target - travelled) / segment)
            target += step
        }
        travelled += segment
    }

    out += path.last()
    return out
}

/** How far [point] lies off the line from [from] to [to]; signed, in points. */
private fun strayFromChord(point: Offset, from: Offset, to: Offset): Float {
    val span = to - from
    val length = span.getDistance()
    if (length <= 0f) return 0f
    return ((point - from).x * span.y - (point - from).y * span.x) / length
}

/**
 * Whether this tool waits until the finger lifts before correcting.
 *
 * The cloud can be shown as it will be committed, because its scallops are local:
 * adding to the ring extends it and leaves what came before alone.
 *
 * A curve cannot. Correcting it means deciding what the *whole* stroke meant —
 * where the bends are, how deep each one goes — and every one of those answers
 * changes as the stroke grows. Re-deciding thirty times a second made the line
 * thrash about under the finger, which is worse than useless: you cannot aim at
 * something that will not hold still.
 *
 * So the trace is shown as drawn and corrected on lift. The correction follows the
 * trace closely, so the snap is small — and a small snap at the end beats a line
 * that never settles.
 */
val AnnotationTool.correctsOnRelease: Boolean
    get() = this == AnnotationTool.Curve || this == AnnotationTool.CurvedArrow

/** As [correctsOnRelease], for the screenshot editor's own tools. */
val MarkupTool.correctsOnRelease: Boolean
    get() = this == MarkupTool.Curve || this == MarkupTool.CurvedArrow

/** How many points a curve is sampled into. Enough that no join is visible. */
const val CURVE_SEGMENTS = 48

/** No span of the spline gets fewer than this, however many bends there are. */
private const val MINIMUM_SEGMENTS = 8

/**
 * How many even steps the trace is walked in, to find its changes of direction.
 *
 * Enough to see a real bend, few enough that hand-shake does not survive the walk.
 * Raising it finds smaller bends and more imaginary ones.
 */
private const val TURN_SAMPLES = 24

/**
 * How far the line must bow, as a fraction of its own length, to count as bending
 * one way rather than the other.
 *
 * Relative because it is separating a hand from an intention, and both scale with
 * the stroke: a two-point wobble is shake on a long sweep and a deliberate bend on
 * a short one.
 */
private const val TURN_DEADBAND_FRACTION = 0.03f

/**
 * How much of the stroke a bend has to hold to count as one.
 *
 * A couple of steps leaning the other way at the end of a stroke is the finger
 * lifting off, not a second curve.
 */
private const val MINIMUM_RUN_FRACTION = 0.18f

/** Below this fraction of its own length, a bow is a wobble and not a curve. */
private const val STRAIGHT_ENOUGH = 0.02f

/** Shorter than this and it is a tap, not a curve. In page points. */
private const val MINIMUM_CURVE_POINTS = 4f

/** Arrow head length, as a multiple of the stroke width. Matches the straight one. */
private const val ARROW_HEAD_WIDTHS = 4f

/** Half-angle of the arrow head, in radians — about 25°. */
private const val ARROW_HEAD_ANGLE = 0.44f
