package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin

/**
 * A revision cloud: the ring you drew, redrawn as scallops bulging outward.
 *
 * The drawing-office convention for "look at this bit". You trace roughly around
 * what you mean and the tracing is replaced by a run of equal arcs, because a
 * cloud is a *notation* — it has to look the same whoever drew it, and nobody's
 * hand produces even bumps.
 *
 * The path is closed whether or not you closed it. A cloud that does not meet
 * itself is a squiggle, and the gap is always where the finger lifted rather than
 * anywhere meaningful.
 *
 * Returns one closed polyline, not curves: a PDF ink annotation is a list of
 * points, so the roundness has to be in the sampling. That the reader and the
 * screenshot editor both take it from here is the point — a cloud on a page and a
 * cloud on a picture of that page are the same cloud.
 */
fun cloudOutline(path: List<Offset>, widthPoints: Float): List<Offset> {
    val ring = closedRing(path)
    if (ring.size < 3) return emptyList()

    val perimeter = ring.zipWithNext().sumOf { (a, b) -> (b - a).getDistance().toDouble() }
        .toFloat()
    val bump = bumpLength(widthPoints)
    if (perimeter < bump) return emptyList()

    // Equal bumps that close exactly, rather than a fixed length and a short one
    // left over at the end: the join is where the eye lands, and a runt arc there
    // is the one thing that reads as a mistake rather than as a cloud.
    val count = (perimeter / bump).roundToInt().coerceAtLeast(MINIMUM_BUMPS)
    val anchors = resample(ring, perimeter / count, count)

    // Which way "outward" is, from the ring's winding rather than from its
    // centroid. A centroid sits outside its own shape as soon as the shape is
    // concave — ring a doorway on a plan and half the scallops would point in.
    val sweep = if (signedArea(anchors) >= 0f) 1f else -1f

    val outline = ArrayList<Offset>(count * CLOUD_ARC_SEGMENTS + 1)
    for (index in anchors.indices) {
        val from = anchors[index]
        val to = anchors[(index + 1) % anchors.size]
        outline += arcPoints(from, to, sweep)
    }
    // Closed by repeating the first point rather than by letting the last arc run
    // on: cos of a full turn is not exactly cos of nothing, and the float between
    // them is a hairline notch at the join.
    outline += anchors.first()

    return outline
}

/**
 * How long one scallop's chord is, in page points.
 *
 * Tied to the nib width, so the one size control means the same thing it always
 * did — a fine cloud is finely scalloped and a heavy one is bold. The floor is
 * for the finest nib: bumps smaller than this stop reading as bumps and the cloud
 * turns back into the wobbly line it was drawn as.
 */
fun bumpLength(widthPoints: Float): Float =
    (widthPoints * CLOUD_BUMP_WIDTHS).coerceAtLeast(MINIMUM_BUMP_POINTS)

/** The drawn path, cleaned of repeats and joined back to where it started. */
private fun closedRing(path: List<Offset>): List<Offset> {
    val clean = ArrayList<Offset>(path.size + 1)
    path.forEach { point ->
        if (clean.isEmpty() || (point - clean.last()).getDistance() > 0f) clean += point
    }
    if (clean.size >= 2 && (clean.first() - clean.last()).getDistance() > 0f) {
        clean += clean.first()
    }
    return clean
}

/**
 * [count] points spaced [step] apart along the ring.
 *
 * By length rather than by point, so the scallops are even however the hand moved
 * — a slow corner leaves a hundred touch samples in a few points of travel, and
 * spacing by index would spend half the cloud's bumps on it.
 */
private fun resample(ring: List<Offset>, step: Float, count: Int): List<Offset> {
    val out = ArrayList<Offset>(count)
    out += ring.first()

    var travelled = 0f
    var target = step

    for ((from, to) in ring.zipWithNext()) {
        val segment = (to - from).getDistance()
        if (segment <= 0f) continue

        while (travelled + segment >= target && out.size < count) {
            out += from + (to - from) * ((target - travelled) / segment)
            target += step
        }
        travelled += segment
    }

    // Rounding can leave the walk a hair short of the last anchor.
    while (out.size < count) out += ring.last()
    return out
}

/**
 * Twice the area the ring encloses, signed by which way it was traced.
 *
 * Positive when the ring was drawn clockwise on screen — y grows downward here,
 * so that is the opposite of the sign this returns on graph paper.
 */
private fun signedArea(ring: List<Offset>): Float {
    var total = 0f
    for (index in ring.indices) {
        val a = ring[index]
        val b = ring[(index + 1) % ring.size]
        total += a.x * b.y - b.x * a.y
    }
    return total
}

/**
 * One scallop: a half-circle on the chord from [from] to [to].
 *
 * A half circle rather than a shallower arc because that is the cloud everyone
 * has seen — consecutive semicircles meet tangentially, so the outline reads as
 * one scalloped edge instead of a string of separate bites.
 *
 * The end point is left off; the next arc starts there, and repeating it would
 * put a doubled point at every join.
 */
private fun arcPoints(from: Offset, to: Offset, sweep: Float): List<Offset> {
    val centre = (from + to) / 2f
    val radius = (to - from).getDistance() / 2f
    val start = atan2(from.y - centre.y, from.x - centre.x)

    return (0 until CLOUD_ARC_SEGMENTS).map { step ->
        val angle = start + sweep * PI.toFloat() * step / CLOUD_ARC_SEGMENTS
        Offset(centre.x + radius * cos(angle), centre.y + radius * sin(angle))
    }
}

/** Scallop chord, as a multiple of the nib width. */
private const val CLOUD_BUMP_WIDTHS = 8f

/** No scallop shorter than this, in page points, however fine the nib. */
private const val MINIMUM_BUMP_POINTS = 6f

/** Fewer than this and it is a flower, not a cloud. */
private const val MINIMUM_BUMPS = 4

/**
 * How many points each half-circle is sampled into.
 *
 * Public because it is the stride of the outline: the point halfway through each
 * run of this many is that scallop's apex, which is what tells a cloud bulging
 * outward from one bulging in.
 */
const val CLOUD_ARC_SEGMENTS = 12
