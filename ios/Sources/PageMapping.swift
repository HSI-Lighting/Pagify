import CoreGraphics
import Foundation

/// Between a page's own points and the pixels it is drawn at. Android's
/// `core/PageMapping.kt`.
///
/// One type for both places a page is drawn. The list draws it at the top-left of
/// its row; the magnified view draws it wherever the reader has panned it to. The
/// only difference is the origin — and it is exactly that difference that made
/// the tools stop working when zoomed, because the drawing layer there assumed an
/// origin of zero.
struct PageMapping {
    /// Pixels per page point.
    var scale: CGFloat
    /// Where the page's top-left corner actually sits on screen.
    var origin: CGPoint = .zero
    var quarterTurns: Int = 0
    /// The page's own size, upright — the space marks are stored in.
    var pageWidthPoints: CGFloat = 0
    var pageHeightPoints: CGFloat = 0

    static let unmeasured = PageMapping(scale: 0)

    /// Whether anything can be placed at all; a page not yet measured cannot.
    var isUsable: Bool { scale > 0 }

    private var turns: Int { ((quarterTurns % 4) + 4) % 4 }

    /// A page point, in the pixels of the page as drawn.
    func toScreen(_ point: CGPoint) -> CGPoint {
        let turned = turn(point)
        return CGPoint(x: turned.x * scale + origin.x, y: turned.y * scale + origin.y)
    }

    /// A touch, in the page's own points.
    func toPage(_ position: CGPoint) -> CGPoint {
        guard isUsable else { return .zero }
        return unturn(CGPoint(x: (position.x - origin.x) / scale,
                              y: (position.y - origin.y) / scale))
    }

    /// A page point, pulled back onto the page if it has left it.
    ///
    /// Held at the edge rather than dropped: a stroke taken past the margin should
    /// run along it, which is what every drawing tool does and what a hand
    /// expects. Dropping the points would break the stroke into pieces, and
    /// discarding the gesture would lose a mark someone meant to make.
    ///
    /// Clamping on the way *in* is what keeps the file honest — clipping the
    /// drawing alone would leave the points in the annotation, off the page, where
    /// another viewer might well show them.
    func clampToPage(_ point: CGPoint) -> CGPoint {
        guard pageWidthPoints > 0, pageHeightPoints > 0 else { return point }
        return CGPoint(x: min(max(point.x, 0), pageWidthPoints),
                       y: min(max(point.y, 0), pageHeightPoints))
    }

    private func turn(_ point: CGPoint) -> CGPoint {
        switch turns {
        case 1: return CGPoint(x: pageHeightPoints - point.y, y: point.x)
        case 2: return CGPoint(x: pageWidthPoints - point.x, y: pageHeightPoints - point.y)
        case 3: return CGPoint(x: point.y, y: pageWidthPoints - point.x)
        default: return point
        }
    }

    private func unturn(_ point: CGPoint) -> CGPoint {
        switch turns {
        case 1: return CGPoint(x: point.y, y: pageHeightPoints - point.x)
        case 2: return CGPoint(x: pageWidthPoints - point.x, y: pageHeightPoints - point.y)
        case 3: return CGPoint(x: pageWidthPoints - point.y, y: point.x)
        default: return point
        }
    }
}
