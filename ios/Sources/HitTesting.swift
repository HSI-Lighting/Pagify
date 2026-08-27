import CoreGraphics
import Foundation

// Which mark a finger is pointing at. Android's `Annotation.isHitBy` in
// `core/Annotation.kt`.
//
// Per shape, never by bounding box. A bounding box finds a highlight from the
// gap between two of its lines, finds a circle from its empty middle, and finds
// a caption from a corner of the page it does not occupy — so the eraser takes
// the wrong mark, which is the one mistake an eraser must not make.

/// How far from a mark still counts as touching it, in page points.
let eraserTouchRadius: CGFloat = 14

extension WireAnnotation {
    func isHitBy(_ point: CGPoint, tolerance: CGFloat) -> Bool {
        switch self {
        case .highlight(let rects, _):
            // **Any** rect, not their union: a tap in the gap between two covered
            // lines is a miss, and unioning them would make it a hit.
            return rects.contains { $0.cgRect.insetBy(dx: -tolerance, dy: -tolerance).contains(point) }

        case .ink(let strokes, _, let width):
            // Along the line, not inside its box — so a tap in the middle of a
            // drawn circle misses it, and a tap in a dash gap still finds it.
            let reach = tolerance + width / 2
            return strokes.contains { isNear(point, polyline: $0, within: reach) }

        case .note(let rect, _, _):
            let anchor = CGPoint(x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2)
            return hypot(point.x - anchor.x, point.y - anchor.y)
                <= tolerance + AnnotationMetrics.noteMarkerRadius

        case .text(let mark):
            // Three cases. A framed caption is caught by its ring, a block by its
            // box, and a plain single line by its *baseline* — a line of type is
            // mostly empty space, and its box would swallow taps meant for
            // whatever is drawn beside it.
            if mark.frame != .none {
                return mark.textFrameBounds().cgRect
                    .insetBy(dx: -tolerance, dy: -tolerance).contains(point)
            }
            if mark.isMultiLine {
                return mark.textBlockBounds().cgRect
                    .insetBy(dx: -tolerance, dy: -tolerance).contains(point)
            }
            return isNear(point, polyline: mark.path, within: tolerance + mark.size)
        }
    }
}

/// Whether `point` is within `reach` of any segment of the polyline.
func isNear(_ point: CGPoint, polyline: [CGPoint], within reach: CGFloat) -> Bool {
    guard polyline.count >= 2 else {
        guard let only = polyline.first else { return false }
        return hypot(point.x - only.x, point.y - only.y) <= reach
    }
    for (from, to) in zip(polyline, polyline.dropFirst()) where
        distance(from: point, toSegment: from, to) <= reach {
        return true
    }
    return false
}

/// Distance from a point to a line segment, with the degenerate segment falling
/// back to a point distance rather than dividing by zero.
func distance(from point: CGPoint, toSegment a: CGPoint, _ b: CGPoint) -> CGFloat {
    let dx = b.x - a.x
    let dy = b.y - a.y
    let lengthSquared = dx * dx + dy * dy
    guard lengthSquared > 0 else { return hypot(point.x - a.x, point.y - a.y) }

    var t = ((point.x - a.x) * dx + (point.y - a.y) * dy) / lengthSquared
    t = min(max(t, 0), 1)
    return hypot(point.x - (a.x + t * dx), point.y - (a.y + t * dy))
}
