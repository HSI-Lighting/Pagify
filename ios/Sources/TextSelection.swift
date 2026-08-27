import CoreGraphics
import Foundation

/// One run of text on a page, with the box it occupies. The engine's
/// `TextSegment`, in page points from the top left.
struct TextSegment {
    var left: CGFloat
    var top: CGFloat
    var right: CGFloat
    var bottom: CGFloat
    var text: String

    var rect: PageRect { PageRect(left: left, top: top, right: right, bottom: bottom) }

    func contains(_ point: CGPoint) -> Bool {
        point.x >= left && point.x <= right && point.y >= top && point.y <= bottom
    }
}

/// Turning a drag across a page into the runs of text it covered. Android's
/// `core/TextSelection.kt`.
///
/// This is what makes the highlighter a highlighter. Without it a sweep across
/// two lines of prose paints one rectangle the height of the drag — including the
/// white space, including the margins, including whatever was in between.
enum TextSelection {
    /// Below this a rect is a sliver no one asked for, usually a clipped edge.
    private static let minimumWidthPoints: CGFloat = 0.5
    /// How much more a point missing vertically counts than missing sideways.
    private static let verticalWeight: CGFloat = 4

    /// The runs between two points, trimmed at both ends.
    static func rectsBetween(_ segments: [TextSegment],
                             anchor: CGPoint, focus: CGPoint) -> [PageRect] {
        guard let anchorIndex = indexNear(segments, anchor),
              let focusIndex = indexNear(segments, focus) else { return [] }

        // Walked in reading order, so the point that trims the left edge is
        // whichever came first in the *text* — not whichever finger went down
        // first.
        let forwards = anchorIndex <= focusIndex
        let first = forwards ? anchorIndex : focusIndex
        let last = forwards ? focusIndex : anchorIndex
        let startX = forwards ? anchor.x : focus.x
        let endX = forwards ? focus.x : anchor.x

        var rects: [PageRect] = []
        rects.reserveCapacity(last - first + 1)

        for index in first...last {
            let segment = segments[index]
            var left = segment.left
            var right = segment.right
            if index == first { left = max(left, startX) }
            if index == last { right = min(right, endX) }

            // An empty intersection means the drag never covered this run
            // horizontally, and the run is simply not selected.
            //
            // Normalising the two edges with min/max instead would silently turn
            // "no overlap" — left past right — into a positive-width rect
            // spanning the gap between them, which is how a drag inside one
            // column paints a band reaching into the next.
            if right - left <= minimumWidthPoints { continue }

            rects.append(PageRect(left: left, top: segment.top,
                                  right: right, bottom: segment.bottom))
        }
        return rects
    }

    /// The run nearest `point`, preferring one it lands inside.
    ///
    /// Vertical distance is weighted, because a touch that misses the text is far
    /// more likely to have missed it along the line it was aiming at than to have
    /// meant the line above or below.
    static func indexNear(_ segments: [TextSegment], _ point: CGPoint) -> Int? {
        var best = -1
        var bestScore = CGFloat.greatestFiniteMagnitude

        for (index, segment) in segments.enumerated() {
            let dy: CGFloat
            if point.y < segment.top { dy = segment.top - point.y }
            else if point.y > segment.bottom { dy = point.y - segment.bottom }
            else { dy = 0 }

            let dx: CGFloat
            if point.x < segment.left { dx = segment.left - point.x }
            else if point.x > segment.right { dx = point.x - segment.right }
            else { dx = 0 }

            let score = dy * verticalWeight + dx
            if score < bestScore {
                bestScore = score
                best = index
            }
        }
        return best >= 0 ? best : nil
    }
}

/// Decode `getTextSegmentsJson`.
func decodeTextSegments(_ json: String) -> [TextSegment] {
    guard let data = json.data(using: .utf8),
          let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] else {
        return []
    }
    return items.map {
        TextSegment(left: $0["left"] as? CGFloat ?? 0,
                    top: $0["top"] as? CGFloat ?? 0,
                    right: $0["right"] as? CGFloat ?? 0,
                    bottom: $0["bottom"] as? CGFloat ?? 0,
                    text: $0["text"] as? String ?? "")
    }
}
