import CoreGraphics
import Foundation

/// The line types. Android's `MarkupStyle` in `core/Markup.kt`.
///
/// Genuinely shared between the reader and the capture editor — a dashed line on
/// a page and a dashed line on a picture of that page are the same line, and two
/// copies of these proportions would be two that agreed until somebody changed
/// one.
enum MarkupStyle: String, CaseIterable, Identifiable {
    case solid
    case dash1
    case dash2
    case centerline1
    case centerline2

    var id: String { rawValue }

    var label: String {
        switch self {
        case .solid: return "Solid"
        case .dash1: return "Dash-1"
        case .dash2: return "Dash-2"
        case .centerline1: return "Centerline-1"
        case .centerline2: return "Centerline-2"
        }
    }

    var isBroken: Bool { self != .solid }

    /// The same proportions the engine and the capture editor draw with.
    ///
    /// A dot is a segment of **almost no length** — `w * 0.01`, not a hundredth
    /// of that again and not thirty-five times it. Ink has round caps, so a dot
    /// draws as a nib rather than as nothing; make it longer and a centreline
    /// reads as a second dash.
    func dashPattern(width: CGFloat) -> [CGFloat] {
        let w = max(width, 1)
        let dot = w * 0.01
        switch self {
        case .solid: return []
        case .dash1: return [w * 4, w * 3]
        case .dash2: return [w * 9, w * 4]
        case .centerline1: return [w * 9, w * 3, dot, w * 3]
        case .centerline2: return [w * 9, w * 3, dot, w * 3, dot, w * 3]
        }
    }
}
