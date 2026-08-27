import SwiftUI

// The glyphs, ported from Android rather than substituted for.
//
// Seven of these are drawings in `CurveIcons.kt` and `EraserIcon.kt`, not system
// symbols, and the substitutes the port reached for were wrong in ways that
// mattered: `cloud` and `cloudText` came out byte-identical, so two tools showed
// one picture, and `pen` and `curve` were swapped.
//
// The coordinates below are Android's, unchanged, in the 24-unit space its
// `ImageVector`s declare. They are scaled to whatever frame the caller asks for
// rather than redrawn at each size, because a glyph redrawn by eye at 14pt is how
// the pair of curve icons stopped being the same arc.

/// One glyph, without saying which tool wants it.
///
/// The ribbon is shared by the reader and the capture editor and neither enum is
/// visible to it, so a slot carries its picture the way Android's `RibbonTool`
/// carries an `ImageVector` — as a value, not as a tool identity to look up.
enum RibbonIcon: Equatable {
    case system(String)
    case drawn(DrawnGlyph)
}

/// The seven that have no SF Symbol worth borrowing.
enum DrawnGlyph: Equatable {
    case curvedLine
    case curvedArrow
    case curvedText
    case cloudText
    case boxText
    case ellipseText
    case eraser
}

extension AnnotationTool {
    /// The SF Symbol for this tool, or nil where it is drawn instead.
    var systemImage: String? {
        switch self {
        // A loose squiggle — deliberately the mark itself rather than a brush.
        case .pen: return "scribble.variable"
        // Horizontal, not diagonal.
        case .line: return "minus"
        case .arrow: return "arrow.right"
        case .rectangle: return "square"
        case .ellipse: return "circle"
        case .cloud: return "cloud"
        case .highlight: return "highlighter"
        case .text: return "textformat"
        case .note: return "note.text"
        case .signature: return "signature"
        case .snapshot: return "viewfinder"
        case .none: return nil
        // Drawn, not symbolised.
        case .curve, .curvedArrow, .curvedText, .cloudText, .boxText, .ellipseText, .eraser:
            return nil
        }
    }

    /// What the ribbon puts in this tool's slot.
    var ribbonIcon: RibbonIcon {
        switch self {
        case .curve: return .drawn(.curvedLine)
        case .curvedArrow: return .drawn(.curvedArrow)
        case .curvedText: return .drawn(.curvedText)
        case .cloudText: return .drawn(.cloudText)
        case .boxText: return .drawn(.boxText)
        case .ellipseText: return .drawn(.ellipseText)
        case .eraser: return .drawn(.eraser)
        default: return .system(systemImage ?? "questionmark")
        }
    }
}

/// One glyph, symbol or drawing, tinted by whatever the caller sets.
struct RibbonGlyphView: View {
    let icon: RibbonIcon
    var size: CGFloat = 20

    var body: some View {
        switch icon {
        case .system(let symbol):
            Image(systemName: symbol)
                .font(.system(size: size * 0.85, weight: .medium))
        case .drawn(.curvedLine):
            CurvedLineIcon().glyphStroke(size)
        case .drawn(.curvedArrow):
            CurvedArrowIcon().glyphStroke(size)
        case .drawn(.curvedText):
            CurvedTextIcon().glyphStroke(size)
        case .drawn(.cloudText):
            FramedTextIcon(frame: .cloud, size: size)
        case .drawn(.boxText):
            FramedTextIcon(frame: .box, size: size)
        case .drawn(.ellipseText):
            FramedTextIcon(frame: .ellipse, size: size)
        case .drawn(.eraser):
            EraserIcon().glyphStroke(size)
        }
    }
}

/// One tool's glyph.
struct ToolIcon: View {
    let tool: AnnotationTool
    var size: CGFloat = 20

    var body: some View {
        RibbonGlyphView(icon: tool.ribbonIcon, size: size)
    }
}

extension Shape {
    /// The weight Android gives every drawn glyph so it sits beside the Material
    /// outline icons without looking lighter or heavier than them.
    func glyphStroke(_ size: CGFloat, width: CGFloat = 1.9,
                     cap: CGLineCap = .round, join: CGLineJoin = .round) -> some View {
        stroke(style: StrokeStyle(lineWidth: width * size / 24, lineCap: cap, lineJoin: join))
            .frame(width: size, height: size)
    }
}

/// How wide one of Android's 24-unit icon coordinates is here.
private func iconScale(_ rect: CGRect) -> CGFloat { min(rect.width, rect.height) / 24 }

/// One smooth bend, rising to the right.
///
/// The shape a curved line is reached for to draw — round an obstacle, or from a
/// note to the thing it is about.
struct CurvedLineIcon: Shape {
    func path(in rect: CGRect) -> Path {
        let s = iconScale(rect)
        var path = Path()
        path.move(to: CGPoint(x: 3 * s, y: 17.5 * s))
        path.addQuadCurve(to: CGPoint(x: 20.5 * s, y: 7.5 * s),
                          control: CGPoint(x: 9 * s, y: 4.5 * s))
        return path
    }
}

/// The same arc with a head on it, so the head is the only difference between the
/// two glyphs — which is the only difference between the two tools.
struct CurvedArrowIcon: Shape {
    func path(in rect: CGRect) -> Path {
        let s = iconScale(rect)
        var path = CurvedLineIcon().path(in: rect)
        // Barbs opening back along the direction the curve is travelling when it
        // arrives, not along the straight line between the ends — on a bend this
        // deep that would sit visibly askew.
        path.move(to: CGPoint(x: 14.6 * s, y: 8.9 * s))
        path.addLine(to: CGPoint(x: 20.5 * s, y: 7.5 * s))
        path.addLine(to: CGPoint(x: 16.2 * s, y: 3.3 * s))
        return path
    }
}

/// Letters on a curve: an arc with three strokes standing square to it.
///
/// Actual letters would be a smudge at this size; what the glyph has to say is
/// "writing, following a line", and the three uprights say it because each one is
/// perpendicular to the baseline under it.
struct CurvedTextIcon: Shape {
    func path(in rect: CGRect) -> Path {
        let s = iconScale(rect)
        var path = Path()
        path.move(to: CGPoint(x: 2.5 * s, y: 16.5 * s))
        path.addQuadCurve(to: CGPoint(x: 21.5 * s, y: 16.5 * s),
                          control: CGPoint(x: 12 * s, y: 6 * s))

        for (x, y, degrees) in [(5.6, 12.6, -38.0), (12.0, 9.6, 0.0), (18.4, 12.6, 38.0)] {
            let radians = degrees * .pi / 180
            let dx = sin(radians) * letterHeight
            let dy = cos(radians) * letterHeight
            path.move(to: CGPoint(x: x * s, y: y * s))
            path.addLine(to: CGPoint(x: (x - dx) * s, y: (y - dy) * s))
        }
        return path
    }

    /// How tall the letters stand off the baseline, in the icon's own units.
    private var letterHeight: Double { 5 }
}

/// Words inside a frame — the ring only, at the lighter weight.
///
/// Separate from the letter so the two can be stroked at their own widths: the
/// frame is a border, not the subject, and drawing both at 1.9 makes the glyph
/// read as a box with something in it rather than as text.
struct FramedTextRing: Shape {
    let frame: TextFrame

    func path(in rect: CGRect) -> Path {
        let s = iconScale(rect)
        var path = Path()

        switch frame {
        case .box:
            path.move(to: CGPoint(x: 2.5 * s, y: 5.5 * s))
            path.addLine(to: CGPoint(x: 21.5 * s, y: 5.5 * s))
            path.addLine(to: CGPoint(x: 21.5 * s, y: 18.5 * s))
            path.addLine(to: CGPoint(x: 2.5 * s, y: 18.5 * s))
            path.closeSubpath()
        case .ellipse:
            // Android draws this as two relative arcs of rx 9.5, ry 6.5 about
            // (12, 12), which is exactly this ellipse.
            path.addEllipse(in: CGRect(x: 2.5 * s, y: 5.5 * s, width: 19 * s, height: 13 * s))
        case .cloud:
            // Built by the same `cloudOutline` that draws the real notation, at
            // icon scale, so the glyph cannot drift away from what it stands for.
            let box = [CGPoint(x: 3.5, y: 7.5), CGPoint(x: 20.5, y: 7.5),
                       CGPoint(x: 20.5, y: 16.5), CGPoint(x: 3.5, y: 16.5)]
            let ring = cloudOutline(box, width: 0.7)
            if let first = ring.first, ring.count >= 2 {
                path.move(to: CGPoint(x: first.x * s, y: first.y * s))
                for point in ring.dropFirst() {
                    path.addLine(to: CGPoint(x: point.x * s, y: point.y * s))
                }
            }
        case .none:
            break
        }
        return path
    }
}

/// A capital T inside a frame: enough to say "words", and the one letter that
/// reads at this size.
struct FramedTextLetter: Shape {
    let frame: TextFrame

    func path(in rect: CGRect) -> Path {
        let s = iconScale(rect)
        // The cloud sits lower and shallower than the box and the ellipse do,
        // because the scallops eat into the room the letter has.
        let top: CGFloat = frame == .cloud ? 10.2 : 9.8
        let foot: CGFloat = frame == .cloud ? 14.2 : 14.4

        var path = Path()
        path.move(to: CGPoint(x: 9 * s, y: top * s))
        path.addLine(to: CGPoint(x: 15 * s, y: top * s))
        path.move(to: CGPoint(x: 12 * s, y: top * s))
        path.addLine(to: CGPoint(x: 12 * s, y: foot * s))
        return path
    }
}

/// Words inside a ring: the three framed-text tools, which differ only in what the
/// ring is. Drawing them from one pair of shapes is what stops two of them slowly
/// stopping matching.
struct FramedTextIcon: View {
    let frame: TextFrame
    var size: CGFloat = 20

    var body: some View {
        ZStack {
            // The cloud's scallops are a chain of short segments and need the round
            // cap to close; the box and the ellipse are one continuous outline and
            // take Android's default butt cap.
            FramedTextRing(frame: frame)
                .glyphStroke(size, width: 1.2, cap: frame == .cloud ? .round : .butt, join: .miter)
            FramedTextLetter(frame: frame).glyphStroke(size)
        }
        .frame(width: size, height: size)
    }
}

/// An eraser: a long block laid over at about forty degrees, a ferrule across it a
/// third of the way up, and the paper it is being dragged along running off right.
///
/// The ferrule is what makes it an eraser rather than a tilted box — without it the
/// glyph is a parallelogram and reads as nothing at all.
struct EraserIcon: Shape {
    func path(in rect: CGRect) -> Path {
        let s = iconScale(rect)
        var path = Path()

        path.move(to: CGPoint(x: 2.7 * s, y: 14.2 * s))
        path.addLine(to: CGPoint(x: 16.3 * s, y: 2.4 * s))
        path.addLine(to: CGPoint(x: 21.3 * s, y: 8.2 * s))
        path.addLine(to: CGPoint(x: 7.7 * s, y: 20.0 * s))
        path.closeSubpath()

        path.move(to: CGPoint(x: 7.5 * s, y: 9.4 * s))
        path.addLine(to: CGPoint(x: 12.5 * s, y: 15.2 * s))

        path.move(to: CGPoint(x: 7.7 * s, y: 20.0 * s))
        path.addLine(to: CGPoint(x: 21.3 * s, y: 20.0 * s))
        return path
    }
}

/// The collapsed drawing slot uses its **own, shorter** map — it reflects the
/// armed tool, and everything that is not one of the five named shapes shows the
/// brush and reads "Pen". Deliberately not `AnnotationTool.systemImage`.
enum CollapsedDrawingSlot {
    static func symbol(for tool: AnnotationTool) -> String {
        switch tool {
        case .line: return "minus"
        case .arrow: return "arrow.right"
        case .rectangle: return "square"
        case .ellipse: return "circle"
        case .cloud: return "cloud"
        default: return "paintbrush"
        }
    }

    static func label(for tool: AnnotationTool) -> String {
        switch tool {
        case .line, .arrow, .rectangle, .ellipse, .cloud: return tool.label
        default: return "Pen"
        }
    }
}
