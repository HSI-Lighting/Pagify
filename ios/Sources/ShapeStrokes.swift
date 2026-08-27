import CoreGraphics
import Foundation

// Turning a drag into the strokes a shape is made of, in page points. A port of
// Android's `core/ShapeStrokes.kt` and `core/CloudStrokes.kt`.
//
// Kept pure and separate from the layer that captures the drag, because this is
// the part that is wrong in ways that look plausible — an arrow head at the wrong
// end, an ellipse that is a rectangle's inscribed circle rather than its own.

/// Arrow head length, as a multiple of the stroke width.
private let arrowHeadWidths: CGFloat = 4
/// Half-angle of the arrow head, in radians — about 25°.
private let arrowHeadAngle: CGFloat = 0.44
/// How many segments an ellipse is sampled into.
private let ellipseSegments = 96

/// The strokes for a shape dragged from `start` to `end`.
func shapeStrokes(tool: AnnotationTool, start: CGPoint, end: CGPoint,
                  style: MarkupStyle, width: CGFloat) -> [[CGPoint]] {
    let outline: [[CGPoint]]
    switch tool {
    case .line: outline = [[start, end]]
    case .arrow: return arrowStrokes(start: start, end: end, width: width)
        .enumerated()
        // The head is left solid: a barb is a few widths long, so a dash pattern
        // lands on it as a stub or misses it entirely.
        .flatMap { $0.offset == 0 ? dashed($0.element, style: style, width: width) : [$0.element] }
    case .rectangle: outline = [rectangleOutline(start: start, end: end)]
    case .ellipse: outline = [ellipseOutline(start: start, end: end)]
    default: return []
    }
    return outline.flatMap { dashed($0, style: style, width: width) }
}

/// The shaft and the two barbs.
///
/// Three strokes rather than one, so the head keeps its point: a single polyline
/// running out to one barb and back would round off at the tip, and the tip is
/// the part an arrow is for.
private func arrowStrokes(start: CGPoint, end: CGPoint, width: CGFloat) -> [[CGPoint]] {
    let angle = atan2(end.y - start.y, end.x - start.x)
    let head = width * arrowHeadWidths

    return [[start, end]] + [-1.0, 1.0].map { (side: CGFloat) in
        let barb = angle + .pi + side * arrowHeadAngle
        return [end, CGPoint(x: end.x + head * cos(barb), y: end.y + head * sin(barb))]
    }
}

/// A closed rectangle from two dragged corners, whichever way round they came.
private func rectangleOutline(start: CGPoint, end: CGPoint) -> [CGPoint] {
    let r = PageRect(from: start, to: end)
    return [
        CGPoint(x: r.left, y: r.top),
        CGPoint(x: r.right, y: r.top),
        CGPoint(x: r.right, y: r.bottom),
        CGPoint(x: r.left, y: r.bottom),
        CGPoint(x: r.left, y: r.top),
    ]
}

/// An ellipse inscribed in the dragged rectangle, as a closed polyline.
///
/// Sampled rather than drawn with curves because ink has no curves: a PDF ink
/// annotation is a list of points, so the smoothness has to be in the sampling.
private func ellipseOutline(start: CGPoint, end: CGPoint) -> [CGPoint] {
    let cx = (start.x + end.x) / 2
    let cy = (start.y + end.y) / 2
    let rx = abs(end.x - start.x) / 2
    let ry = abs(end.y - start.y) / 2

    // Closed by repeating the first point rather than by walking a full turn:
    // cos(2pi) is not exactly cos(0), and the float that separates them leaves a
    // hairline gap where the ellipse joins itself.
    let ring = (0..<ellipseSegments).map { step -> CGPoint in
        let angle = CGFloat(step) / CGFloat(ellipseSegments) * 2 * .pi
        return CGPoint(x: cx + rx * cos(angle), y: cy + ry * sin(angle))
    }
    return ring + [ring[0]]
}

/// Cut a polyline into the dashes its line type asks for.
///
/// Walks the line by length rather than by point, so a dash falls where it should
/// on a curve as well as on a straight run — an ellipse is a hundred short
/// segments, and dashing per segment would put a dash on each one.
///
/// A solid line comes back as itself, in one piece: the common case pays nothing.
func dashed(_ points: [CGPoint], style: MarkupStyle, width: CGFloat) -> [[CGPoint]] {
    let pattern = style.dashPattern(width: width)
    guard !pattern.isEmpty, points.count >= 2 else { return [points] }

    var strokes: [[CGPoint]] = []
    var current: [CGPoint] = []
    var slot = 0
    var remaining = pattern[0]
    var drawing = true

    var cursor = points[0]
    current.append(cursor)

    for next in points.dropFirst() {
        var segment = hypot(next.x - cursor.x, next.y - cursor.y)
        guard segment > 0 else { continue }

        while segment >= remaining {
            let t = remaining / segment
            let cut = CGPoint(x: cursor.x + (next.x - cursor.x) * t,
                              y: cursor.y + (next.y - cursor.y) * t)
            if drawing {
                current.append(cut)
                if current.count >= 2 { strokes.append(current) }
                current = []
            } else {
                current = [cut]
            }
            drawing.toggle()
            cursor = cut
            segment -= remaining
            slot = (slot + 1) % pattern.count
            remaining = pattern[slot]
        }

        remaining -= segment
        if drawing { current.append(next) }
        cursor = next
    }

    if drawing, current.count >= 2 { strokes.append(current) }
    return strokes
}

// ------------------------------------------------------------------- cloud --

/// Scallop chord, as a multiple of the nib width.
private let cloudBumpWidths: CGFloat = 8
/// No scallop shorter than this, in page points, however fine the nib.
private let minimumBumpPoints: CGFloat = 6
/// Fewer than this and it is a flower, not a cloud.
private let minimumBumps = 4
/// How many points each half-circle is sampled into.
private let cloudArcSegments = 12

func bumpLength(width: CGFloat) -> CGFloat {
    max(width * cloudBumpWidths, minimumBumpPoints)
}

/// A revision cloud around a roughly-traced path.
func cloudOutline(_ path: [CGPoint], width: CGFloat) -> [CGPoint] {
    let ring = closedRing(path)
    guard ring.count >= 3 else { return [] }

    let perimeter = zip(ring, ring.dropFirst())
        .reduce(CGFloat(0)) { $0 + hypot($1.1.x - $1.0.x, $1.1.y - $1.0.y) }
    let bump = bumpLength(width: width)
    guard perimeter >= bump else { return [] }

    // Equal bumps that close exactly, rather than a fixed length and a short one
    // left over at the end: the join is where the eye lands, and a runt arc there
    // is the one thing that reads as a mistake rather than as a cloud.
    let count = max(Int((perimeter / bump).rounded()), minimumBumps)
    let anchors = resample(ring, step: perimeter / CGFloat(count), count: count)

    // Which way "outward" is, from the ring's winding rather than from its
    // centroid. A centroid sits outside its own shape as soon as the shape is
    // concave — ring a doorway on a plan and half the scallops would point in.
    let sweep: CGFloat = signedArea(anchors) >= 0 ? 1 : -1

    var outline: [CGPoint] = []
    outline.reserveCapacity(count * cloudArcSegments + 1)
    for index in anchors.indices {
        outline += arcPoints(from: anchors[index],
                             to: anchors[(index + 1) % anchors.count],
                             sweep: sweep)
    }
    outline.append(anchors[0])
    return outline
}

/// The drawn path, cleaned of repeats and joined back to where it started.
private func closedRing(_ path: [CGPoint]) -> [CGPoint] {
    var clean: [CGPoint] = []
    for point in path {
        if clean.isEmpty || hypot(point.x - clean[clean.count - 1].x,
                                  point.y - clean[clean.count - 1].y) > 0 {
            clean.append(point)
        }
    }
    if clean.count >= 2,
       hypot(clean[0].x - clean[clean.count - 1].x, clean[0].y - clean[clean.count - 1].y) > 0 {
        clean.append(clean[0])
    }
    return clean
}

/// `count` points spaced `step` apart along the ring.
///
/// By length rather than by point, so the scallops are even however the hand
/// moved — a slow corner leaves a hundred touch samples in a few points of
/// travel, and spacing by index would spend half the cloud's bumps on it.
private func resample(_ ring: [CGPoint], step: CGFloat, count: Int) -> [CGPoint] {
    var out: [CGPoint] = [ring[0]]
    var travelled: CGFloat = 0
    var target = step

    for (from, to) in zip(ring, ring.dropFirst()) {
        let segment = hypot(to.x - from.x, to.y - from.y)
        guard segment > 0 else { continue }

        while travelled + segment >= target, out.count < count {
            let t = (target - travelled) / segment
            out.append(CGPoint(x: from.x + (to.x - from.x) * t,
                               y: from.y + (to.y - from.y) * t))
            target += step
        }
        travelled += segment
    }

    // Rounding can leave the walk a hair short of the last anchor.
    while out.count < count { out.append(ring[ring.count - 1]) }
    return out
}

/// Twice the area the ring encloses, signed by which way it was traced.
private func signedArea(_ ring: [CGPoint]) -> CGFloat {
    var total: CGFloat = 0
    for index in ring.indices {
        let a = ring[index]
        let b = ring[(index + 1) % ring.count]
        total += a.x * b.y - b.x * a.y
    }
    return total
}

/// One scallop: a half-circle on the chord.
///
/// A half circle rather than a shallower arc because that is the cloud everyone
/// has seen — consecutive semicircles meet tangentially, so the outline reads as
/// one scalloped edge instead of a string of separate bites.
///
/// The end point is left off; the next arc starts there, and repeating it would
/// put a doubled point at every join.
private func arcPoints(from: CGPoint, to: CGPoint, sweep: CGFloat) -> [CGPoint] {
    let centre = CGPoint(x: (from.x + to.x) / 2, y: (from.y + to.y) / 2)
    let radius = hypot(to.x - from.x, to.y - from.y) / 2
    let start = atan2(from.y - centre.y, from.x - centre.x)

    return (0..<cloudArcSegments).map { step in
        let angle = start + sweep * .pi * CGFloat(step) / CGFloat(cloudArcSegments)
        return CGPoint(x: centre.x + radius * cos(angle), y: centre.y + radius * sin(angle))
    }
}

// ------------------------------------------------------------------ curves --

/// How many segments a traced curve is smoothed into.
private let curveSegments = 48

/// A smooth curve through a traced path.
///
/// A hand-drawn arc looked broken however carefully it was traced, so the path is
/// replaced rather than kept: the finger says *roughly here*, and this says it
/// smoothly. Catmull-Rom because it passes through its control points — a Bézier
/// through the same guides would sag away from where the finger actually went.
func curveThrough(_ path: [CGPoint], segments: Int = curveSegments) -> [CGPoint] {
    let guides = evenlySpaced(path, count: 4)
    guard guides.count >= 4 else { return path }

    // The ends are doubled so the spline starts and finishes at the traced ends
    // rather than short of them.
    var control = [guides[0]] + guides + [guides[guides.count - 1]]
    var out: [CGPoint] = []
    out.reserveCapacity(segments + 1)

    let spans = control.count - 3
    for step in 0...segments {
        let t = CGFloat(step) / CGFloat(segments) * CGFloat(spans)
        let span = min(Int(t), spans - 1)
        out.append(catmullRom(control[span], control[span + 1],
                              control[span + 2], control[span + 3],
                              t - CGFloat(span)))
    }
    control.removeAll()
    return out
}

/// A curve with an arrow head on its far end.
func curvedArrowStrokes(_ path: [CGPoint], width: CGFloat,
                        style: MarkupStyle) -> [[CGPoint]] {
    let curve = curveThrough(path)
    guard curve.count >= 2 else { return [] }

    // The head is aimed along the last segment that has any length: the final
    // sample can land on top of its neighbour, and a zero-length segment gives an
    // angle of nothing and a head pointing due east.
    let tip = curve[curve.count - 1]
    var from = curve[0]
    for point in curve.reversed() where hypot(tip.x - point.x, tip.y - point.y) > width {
        from = point
        break
    }

    let angle = atan2(tip.y - from.y, tip.x - from.x)
    let head = width * arrowHeadWidths
    let barbs = [-1.0, 1.0].map { (side: CGFloat) -> [CGPoint] in
        let barb = angle + .pi + side * arrowHeadAngle
        return [tip, CGPoint(x: tip.x + head * cos(barb), y: tip.y + head * sin(barb))]
    }

    return dashed(curve, style: style, width: width) + barbs
}

/// `count` points spaced evenly along the path by length.
///
/// By length rather than by index, because a slow corner leaves a hundred touch
/// samples in a few points of travel and would drag every guide into it.
private func evenlySpaced(_ path: [CGPoint], count: Int) -> [CGPoint] {
    let clean = path.enumerated().filter { index, point in
        index == 0 || hypot(point.x - path[index - 1].x, point.y - path[index - 1].y) > 0
    }.map(\.element)
    guard clean.count >= 2 else { return clean }

    let total = zip(clean, clean.dropFirst())
        .reduce(CGFloat(0)) { $0 + hypot($1.1.x - $1.0.x, $1.1.y - $1.0.y) }
    guard total > 0 else { return clean }

    let step = total / CGFloat(count - 1)
    var out: [CGPoint] = [clean[0]]
    var travelled: CGFloat = 0
    var target = step

    for (from, to) in zip(clean, clean.dropFirst()) {
        let segment = hypot(to.x - from.x, to.y - from.y)
        guard segment > 0 else { continue }
        while travelled + segment >= target, out.count < count {
            let t = (target - travelled) / segment
            out.append(CGPoint(x: from.x + (to.x - from.x) * t,
                               y: from.y + (to.y - from.y) * t))
            target += step
        }
        travelled += segment
    }
    while out.count < count { out.append(clean[clean.count - 1]) }
    return out
}

private func catmullRom(_ p0: CGPoint, _ p1: CGPoint, _ p2: CGPoint, _ p3: CGPoint,
                        _ t: CGFloat) -> CGPoint {
    let t2 = t * t
    let t3 = t2 * t
    func axis(_ a: CGFloat, _ b: CGFloat, _ c: CGFloat, _ d: CGFloat) -> CGFloat {
        0.5 * ((2 * b) + (-a + c) * t
               + (2 * a - 5 * b + 4 * c - d) * t2
               + (-a + 3 * b - 3 * c + d) * t3)
    }
    return CGPoint(x: axis(p0.x, p1.x, p2.x, p3.x), y: axis(p0.y, p1.y, p2.y, p3.y))
}
