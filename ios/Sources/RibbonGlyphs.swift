import CoreText
import SwiftUI

// The pieces the mark ribbon is built from. Android's glyph and swatch section of
// `ui/components/MarkRibbon.kt`, plus `ColourDot` and `LinePattern`.
//
// Half of these are drawn rather than picked from an icon set, and that is the
// point of the file: a stack of three line weights is not a thing SF Symbols has,
// and a slot whose glyph *is* its value cannot be a label.

// ------------------------------------------------------------------ glyphs --

/// The ink, as a disc. The one slot whose glyph *is* its value.
struct ColourGlyph: View {
    let colour: MarkColor

    var body: some View {
        Circle()
            .fill(Color(colour.cgColor))
            .frame(width: 26, height: 26)
    }
}

/// The weights on offer, stacked, with the one in use picked out.
///
/// Drawn at the weights it actually offers, so the slot is a preview rather than a
/// label: the difference between fine and heavy is the whole point of the control,
/// and three identical lines with a number beside them would say nothing.
struct ThicknessGlyph: View {
    @Environment(\.displayScale) private var displayScale
    let width: CGFloat
    let presets: [CGFloat]
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let plain = PagifyColor.onSurfaceVariant(scheme)
        let heaviest = presets.max() ?? 1

        Canvas { context, size in
            let gap = size.height / CGFloat(presets.count + 1)
            for (index, weight) in presets.enumerated() {
                let y = gap * CGFloat(index + 1)
                var line = Path()
                line.move(to: CGPoint(x: 0, y: y))
                line.addLine(to: CGPoint(x: size.width, y: y))
                context.stroke(
                    line,
                    with: .color(weight == width ? PagifyColor.ribbonAccent : plain),
                    // Scaled against the heaviest on offer rather than by a
                    // constant, because the two ribbons measure different things:
                    // page points in one, a fraction of full strength in the other.
                    style: StrokeStyle(lineWidth: max(weight / heaviest * glyphHeaviestPx / displayScale,
                                                   1.5 / displayScale),
                                       lineCap: .round))
            }
        }
        .frame(width: 30, height: 22)
    }
}

/// Three patterns stacked, the family in use picked out.
///
/// Three rather than five: the two dashes differ only in the length of the dash and
/// the two centre lines only in how many dots, which is a distinction worth making
/// in the panel and not worth making in a 30pt glyph.
struct LineTypeGlyph: View {
    @Environment(\.displayScale) private var displayScale
    let style: MarkupStyle
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let plain = PagifyColor.onSurfaceVariant(scheme)
        let rows: [(MarkupStyle, Bool)] = [
            (.centerline1, style == .centerline1 || style == .centerline2),
            (.dash1, style == .dash1 || style == .dash2),
            (.solid, style == .solid),
        ]

        Canvas { context, size in
            let gap = size.height / CGFloat(rows.count + 1)
            for (index, row) in rows.enumerated() {
                let y = gap * CGFloat(index + 1)
                var line = Path()
                line.move(to: CGPoint(x: 0, y: y))
                line.addLine(to: CGPoint(x: size.width, y: y))
                context.stroke(
                    line,
                    with: .color(row.1 ? PagifyColor.ribbonAccent : plain),
                    style: StrokeStyle(lineWidth: glyphLinePx / displayScale, lineCap: .round,
                                       dash: row.0.dashPattern(width: glyphLinePx / displayScale)))
            }
        }
        .frame(width: 30, height: 22)
    }
}

/// A short line in the pattern it names.
///
/// Drawn rather than three pictures, because the dash lengths have to match what
/// will actually be drawn — a picture of a dashed line that dashes differently from
/// the mark is worse than no picture.
struct LinePattern: View {
    @Environment(\.displayScale) private var displayScale
    let style: MarkupStyle
    let tint: Color
    var width: CGFloat = 40

    var body: some View {
        Canvas { context, size in
            var line = Path()
            line.move(to: CGPoint(x: 0, y: size.height / 2))
            line.addLine(to: CGPoint(x: size.width, y: size.height / 2))
            context.stroke(line, with: .color(tint),
                           style: StrokeStyle(lineWidth: patternWidthPx / displayScale, lineCap: .round,
                                              dash: style.dashPattern(width: patternWidthPx / displayScale)))
        }
        .frame(width: width, height: 12)
    }
}

/// The bend itself, drawn: an arc turning through the amount it stands for.
struct CurveGlyph: View {
    let degrees: CGFloat
    var lit: Bool = true
    var size: CGFloat = 24
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let ink = lit ? PagifyColor.ribbonAccent : PagifyColor.onSurfaceVariant(scheme)

        Canvas { context, canvas in
            let turn = degrees * .pi / 180
            let span = canvas.width * 0.82
            let left = (canvas.width - span) / 2
            let middle = canvas.height / 2
            var path = Path()

            if abs(turn) < 0.02 {
                path.move(to: CGPoint(x: left, y: middle))
                path.addLine(to: CGPoint(x: left + span, y: middle))
            } else {
                // The same arithmetic the baseline uses, so the glyph is a preview
                // of the line rather than a picture of one.
                let radius = span / turn
                let start = -turn / 2
                let steps = 24
                for step in 0...steps {
                    let along = start + turn * CGFloat(step) / CGFloat(steps)
                    let x = left + radius * (sin(along) - sin(start))
                    let y = middle + radius * (cos(start) - cos(along))
                    if step == 0 {
                        path.move(to: CGPoint(x: x, y: y))
                    } else {
                        path.addLine(to: CGPoint(x: x, y: y))
                    }
                }
            }

            context.stroke(path, with: .color(ink),
                           style: StrokeStyle(lineWidth: canvas.width * 0.09, lineCap: .round))
        }
        .frame(width: size, height: size)
    }
}

/// The point size, as the number it is.
///
/// A number rather than a picture, because that is how a size is asked for: nobody
/// chooses type by pointing at a specimen of it, they say twelve. The other slots
/// are glyphs because their answers have no names anybody uses.
struct SizeGlyph: View {
    let sizePoints: CGFloat

    var body: some View {
        Text("\(Int(sizePoints.rounded()))")
            .font(.system(size: 16, weight: .medium))
            .foregroundStyle(PagifyColor.ribbonAccent)
    }
}

/// The face, shown in itself — the one label that can be its own specimen.
struct FontGlyph: View {
    let font: PagifyFont
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Text("Aa")
            .font(RibbonFontFaces.specimen(font, size: 16))
            .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
    }
}

/// Everything in a group at once, with the armed one in the accent.
///
/// The slot has to answer two questions — what does this offer, and what is on —
/// and showing only the armed one answered just the second.
struct GroupGlyph: View {
    let group: [RibbonTool]
    let armed: AnyHashable?

    var body: some View {
        // A slot is a glyph, not a list. Past four members they stop being tellable
        // apart at 14pt, so the preview shows the first four and makes room for
        // whatever is armed when it is not one of them — otherwise picking the fifth
        // tool would leave the row with nothing lit and no way to see what was held.
        let shown = previewOf(group, armed: armed)

        switch shown.count {
        // A group with nothing in it is a caller's mistake, not a layout: an empty
        // slot is a puzzle, a crashed reader is a lost document.
        case 0:
            EmptyView()

        case 1:
            GroupMember(tool: shown[0], armed: armed, size: 26)

        // Turned upright, and only here. A horizontal bar beside a right-pointing
        // arrow is two wide glyphs in a slot with room for two tall ones, and the
        // pair reads as one arrow with a dash in front of it. Stood on end they
        // read as two things.
        case 2:
            HStack(spacing: 2) {
                ForEach(shown) { tool in
                    GroupMember(tool: tool, armed: armed, size: 20)
                        .rotationEffect(.degrees(-90))
                }
            }

        // Two above and one below, rather than three in a row: three glyphs across
        // a 46pt slot leaves each of them too small to tell apart.
        case 3:
            ZStack {
                GroupMember(tool: shown[0], armed: armed, size: 15)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                GroupMember(tool: shown[1], armed: armed, size: 15)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                GroupMember(tool: shown[2], armed: armed, size: 15)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
            }
            .frame(width: 34, height: 34)

        // Four to a corner each. Beyond this a slot stops being a glyph and starts
        // being a list, and the group should be split rather than shrunk further.
        default:
            ZStack {
                GroupMember(tool: shown[0], armed: armed, size: 14)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                GroupMember(tool: shown[1], armed: armed, size: 14)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                GroupMember(tool: shown[2], armed: armed, size: 14)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                GroupMember(tool: shown[3], armed: armed, size: 14)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
            }
            .frame(width: 34, height: 34)
        }
    }
}

private struct GroupMember: View {
    let tool: RibbonTool
    let armed: AnyHashable?
    let size: CGFloat
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        RibbonGlyphView(icon: tool.icon, size: size)
            .foregroundStyle(tool.key == armed
                             ? PagifyColor.ribbonAccent
                             : PagifyColor.onSurfaceVariant(scheme))
    }
}

/// The members a slot previews, at most [previewMembers] of them.
func previewOf(_ group: [RibbonTool], armed: AnyHashable?) -> [RibbonTool] {
    let shown = Array(group.filter(\.inPreview).prefix(previewMembers))
    if shown.isEmpty { return Array(group.prefix(previewMembers)) }
    if shown.contains(where: { $0.key == armed }) { return shown }
    // Whatever is armed always shows, even when it is one of the ones the row does
    // not normally preview: a slot with nothing lit in it looks like no tool is
    // held at all, and that is the one thing the glyph has to say.
    guard let held = group.first(where: { $0.key == armed }) else { return shown }
    return Array(shown.dropLast()) + [held]
}

/// How many members a slot's glyph can show and still be read as a glyph.
let previewMembers = 4

/// How thick the heaviest weight is drawn in a glyph.
/// Android draws these inside a Compose `DrawScope`, where every dimension is a
/// **device pixel** — about a third of a point on a 3x screen. Divided by the
/// display scale below, so a 3x phone reproduces Android's line exactly. Used
/// raw, the three thickness weights merge into one blob and all three line
/// patterns draw as a single long dash.
private let glyphHeaviestPx: CGFloat = 7

/// How thick the line-type patterns are drawn in a glyph.
private let glyphLinePx: CGFloat = 3.5

/// How thick the panel's line patterns are drawn.
private let patternWidthPx: CGFloat = 4

// ----------------------------------------------------------------- swatches --

/// One palette colour, to tap.
///
/// The ring that means "chosen" is the theme's own `primary` rather than the amber
/// every other live choice in this row uses. That is Android's, and it is left
/// alone deliberately: a dot is already a colour, and a second colour ringing it is
/// read as part of the swatch unless it is plainly chrome.
struct ColourDot: View {
    let colour: MarkColor
    let selected: Bool
    let onClick: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: onClick) {
            Circle()
                .fill(Color(colour.cgColor))
                .frame(width: 30, height: 30)
                .overlay(
                    Circle().strokeBorder(
                        selected ? PagifyColor.primary(scheme) : PagifyColor.outlineVariant(scheme),
                        lineWidth: selected ? 3 : 1)
                )
        }
        .buttonStyle(.plain)
    }
}

/// The way out of a fixed palette, and into the wheel.
///
/// Shows the wheel it opens until a colour has been picked from it, and the colour
/// itself afterwards — otherwise choosing a custom colour makes the selection ring
/// vanish from the row and nothing on screen says what is being drawn with.
struct CustomColourSwatch: View {
    let current: MarkColor
    let isCustom: Bool
    let onClick: () -> Void
    var size: CGFloat = 26
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: onClick) {
            ZStack {
                if isCustom {
                    Circle().fill(Color(current.cgColor))
                } else {
                    Circle().fill(AngularGradient(
                        colors: [.red, .yellow, .green, .cyan, .blue, .purple, .red],
                        center: .center))
                }
            }
            .frame(width: size - 4, height: size - 4)
            .frame(width: size, height: size)
            .overlay(
                Circle().strokeBorder(
                    isCustom ? PagifyColor.onSurface(scheme) : PagifyColor.outlineVariant(scheme),
                    lineWidth: isCustom ? 3 : 1)
            )
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Any colour")
    }
}

/// One preset, drawn as a dot in the ink it will draw with.
struct NibDot: View {
    let width: CGFloat
    let heaviest: CGFloat
    let colour: MarkColor
    let selected: Bool
    let onClick: () -> Void

    var body: some View {
        Button(action: onClick) {
            Canvas { context, size in
                // A rank, not a ruler: the presets are page points in one ribbon
                // and a fraction in the other, so the dot is drawn relative to the
                // heaviest on offer rather than at a fixed scale.
                let radius = max(width / heaviest * min(size.width, size.height) / 2, 2)
                let centre = CGPoint(x: size.width / 2, y: size.height / 2)
                context.fill(Path(ellipseIn: CGRect(x: centre.x - radius, y: centre.y - radius,
                                                    width: radius * 2, height: radius * 2)),
                             with: .color(Color(colour.cgColor)))
            }
            .frame(width: 22, height: 22)
            .frame(width: 34, height: 34)
            .overlay(
                Circle().strokeBorder(selected ? PagifyColor.ribbonAccent : .clear, lineWidth: 2)
            )
        }
        .buttonStyle(.plain)
    }
}

/// The corner wedge that says a slot offers more than a tap.
///
/// A right triangle filling the corner, hypotenuse facing up and left, which is
/// what makes it read as an arrow aimed at the corner rather than as a stray dot.
/// Inset far enough to sit **inside** the circle the slot is clipped to — in the
/// true corner what survives the clip is a thin arc that reads as a rendering
/// fault.
struct MoreTick: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Path { path in
            path.move(to: CGPoint(x: hintSize, y: 0))
            path.addLine(to: CGPoint(x: hintSize, y: hintSize))
            path.addLine(to: CGPoint(x: 0, y: hintSize))
            path.closeSubpath()
        }
        .fill(PagifyColor.onSurfaceVariant(scheme))
        .frame(width: hintSize, height: hintSize)
        .padding(.trailing, hintInset)
        .padding(.bottom, hintInset)
    }
}

/// Small enough to be a hint rather than a second icon, large enough to survive
/// being drawn on a dense screen.
private let hintSize: CGFloat = 6

/// Far enough in to clear the circle the slot is clipped to.
private let hintInset: CGFloat = 8

// -------------------------------------------------------------------- faces --

/// The face to draw a font's own name in.
///
/// The bundled file where there is one, so a name written in its own script is
/// drawn by the font it names. Falling back to the system sans would show tofu for
/// half the list — which is the one thing the label was meant to avoid.
enum RibbonFontFaces {
    private static var resolved: [String: String?] = [:]

    static func specimen(_ font: PagifyFont, size: CGFloat) -> Font {
        if let name = postScriptName(font) {
            return .custom(name, fixedSize: size)
        }
        return .system(size: size, weight: font.isBold ? .bold : .regular, design: design(font))
    }

    /// The nearest face the system has, for the standard-14 that have no file.
    private static func design(_ font: PagifyFont) -> Font.Design {
        switch font.wireName {
        case "Times-Roman", "Times-Bold": return .serif
        case "Courier": return .monospaced
        default: return .default
        }
    }

    /// Register the bundled file with Core Text once, and remember what it is
    /// called — `Font.custom` wants a PostScript name, which the file has and the
    /// asset name does not.
    private static func postScriptName(_ font: PagifyFont) -> String? {
        guard let asset = font.asset else { return nil }
        if let cached = resolved[asset] { return cached }

        let stem = (asset as NSString).deletingPathExtension
        let located = Bundle.main.url(forResource: stem, withExtension: "ttf", subdirectory: "fonts")
            ?? Bundle.main.url(forResource: stem, withExtension: "ttf")
        guard let url = located,
              let provider = CGDataProvider(url: url as CFURL),
              let face = CGFont(provider),
              let name = face.postScriptName as String? else {
            // Remembered as a miss rather than left absent, so a face with no file
            // on disk is not looked for again on every recomposition of the list.
            resolved.updateValue(nil, forKey: asset)
            return nil
        }

        // A second registration of a file already registered fails, and that is
        // fine: the face is available either way, and the picker has nothing to
        // report about it.
        CTFontManagerRegisterGraphicsFont(face, nil)
        resolved[asset] = name
        return name
    }
}
