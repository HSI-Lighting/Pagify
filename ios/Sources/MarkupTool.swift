import CoreGraphics
import CoreText
import Foundation
#if canImport(UIKit)
import UIKit
#endif

// What the **capture editor** marks a picture with. Android's `core/Markup.kt`,
// `core/MarkupGesture.kt` and the capture half of `core/TextLayout.kt`.
//
// This is not the reader's model with different words on it. A capture is a
// picture: it has no text layer, no pages, and no file to save marks into, so the
// marks live in **capture-local units** — the picture's own space, origin at its
// top-left. A capture can span two pages and a mark drawn across the join belongs
// to neither of them; nor can the marks be in preview pixels, because the export
// scale can change after a mark is drawn and a mark stored in pixels would land
// somewhere else the moment it did.
//
// Three differences from `AnnotationTool` are load-bearing and were collapsed
// once already:
//
//   * fourteen cases with **no `none` sentinel** — a tool is put down by clearing
//     `MarkupSettings.armed`, so which tool was in hand survives putting it down;
//   * the highlighter's weight is an **intensity**, 0…1, baked into the colour's
//     alpha with `widthPoints = 0`, not a nib width and not a text-snapping
//     selection;
//   * a caption is addressed **by its position in the list**, because a capture
//     mark has no id — the list *is* the drawing, in the order it was made.

/// What the markup toolbar can draw.
///
/// Fourteen, in `Markup.kt` declaration order. The order is load-bearing: the
/// curves sit immediately after the arrow here, where the reader puts them after
/// the box and the circle, so anything that maps an ordinal between the two
/// enums maps it to the wrong tool.
enum MarkupTool: String, CaseIterable, Identifiable {
    case pen
    case line
    case arrow
    case curve
    case curvedArrow
    case rectangle
    case ellipse
    case cloud
    case highlight
    case text
    case curvedText
    case cloudText
    case boxText
    case ellipseText

    var id: String { rawValue }

    /// The names on the ribbon. Word for word the reader's, for the twelve tools
    /// both have — one mark, one name.
    var label: String {
        switch self {
        case .pen: return "Freehand"
        case .line: return "Line"
        case .arrow: return "Arrow"
        case .curve: return "Curved line"
        case .curvedArrow: return "Curved arrow"
        case .rectangle: return "Box"
        case .ellipse: return "Circle"
        case .cloud: return "Cloud"
        case .highlight: return "Highlight"
        case .text: return "Text"
        case .curvedText: return "Curved text"
        case .cloudText: return "Clouded text"
        case .boxText: return "Boxed text"
        case .ellipseText: return "Circled text"
        }
    }

    /// Whether this tool writes words rather than drawing.
    var writesText: Bool {
        switch self {
        case .text, .curvedText, .cloudText, .boxText, .ellipseText: return true
        default: return false
        }
    }

    /// Whether this tool writes on a bent line. As in the reader, the bend is a
    /// setting rather than something traced.
    var bendsText: Bool { self == .curvedText }

    /// What this tool draws around the words it writes, if anything.
    var textFrame: TextFrame {
        switch self {
        case .cloudText: return .cloud
        case .boxText: return .box
        case .ellipseText: return .ellipse
        default: return .none
        }
    }

    /// Whether this tool follows the finger rather than taking two corners.
    ///
    /// Exactly four. The pen keeps the path as drawn; the others replace it — the
    /// cloud with scallops, the curves with the bends they were meant to be.
    var tracesPath: Bool {
        switch self {
        case .pen, .cloud, .curve, .curvedArrow: return true
        default: return false
        }
    }

    /// Whether this tool's shape is dragged corner to corner rather than traced.
    ///
    /// Built from `tracesPath` and `writesText` alone — there is no `draws` here,
    /// because there is no tool on this row that does not make a mark. Text is
    /// excluded because it is placed and then typed, never dragged out.
    var isDragged: Bool { !tracesPath && !writesText }

    /// Whether the traced path is replaced on release by the shape it was meant
    /// to be. Re-deciding what a whole curve meant on every frame made the line
    /// thrash about under the finger, so the trace is shown and corrected on lift.
    var correctsOnRelease: Bool { self == .curve || self == .curvedArrow }

    /// Whether holding still at the end of a stroke asks for the shape recogniser.
    ///
    /// The pen only. A cloud is already the shape it is going to be, and asking
    /// the engine what a hand-traced ring "really" was hands back the ellipse it
    /// was drawn around — which is the one thing a cloud is deliberately not.
    var recognises: Bool { self == .pen }

    /// Whether this tool's weight means the strength of a wash rather than the
    /// width of a nib.
    ///
    /// The highlighter only, and this is the whole of its contract: the number is
    /// 0…1, it ends up in the colour's alpha, and the mark carries no stroke
    /// width at all.
    var isIntensity: Bool { self == .highlight }

    /// Whether this tool draws in a line type. Everything but the highlighter: a
    /// wash has no length to break up.
    var hasLineStyle: Bool { !isIntensity }

    /// Where this tool's slider starts and stops.
    var sizeRange: ClosedRange<CGFloat> { isIntensity ? 0.08...0.85 : 0.6...16 }

    /// What this tool draws at until it is changed.
    var defaultSize: CGFloat { isIntensity ? 0.35 : MarkupMetrics.strokePoints }

    /// The sizes offered as a tap rather than a drag — thin, medium and thick,
    /// always on screen. The slider behind a long press is for when none of the
    /// three is quite right.
    var sizePresets: [CGFloat] {
        isIntensity ? [0.2, 0.4, 0.65] : [1.2, 3, 8]
    }

    /// Whether the slot's glyph previews this one. A group can hold more than a
    /// slot can show, and nothing is hidden by it — the picker lists the whole
    /// group.
    var inPreview: Bool { self != .boxText && self != .ellipseText }

    /// Every tool at its default, for a fresh capture.
    static func defaultSizes() -> [MarkupTool: CGFloat] {
        Dictionary(uniqueKeysWithValues: allCases.map { ($0, $0.defaultSize) })
    }

    /// The tools sharing a place in the row.
    ///
    /// **Four slots, and the highlighter is not among them.** This is the quick-
    /// swap grouping — a long press on a slot offers the others in it — and the
    /// wash shares its question with nothing.
    static let slots: [[MarkupTool]] = [
        [.line, .arrow, .curve, .curvedArrow],
        [.rectangle, .ellipse],
        [.pen, .cloud],
        [.text, .curvedText, .cloudText, .boxText, .ellipseText],
    ]

    /// The tools as the ribbon lays them out.
    ///
    /// **Five**, not four: the highlighter takes a slot of its own, because it is
    /// neither a line nor a way of going round something. Deliberately a
    /// different list from `slots`, and the difference is the whole reason both
    /// exist.
    static let groups: [[MarkupTool]] = [
        [.line, .arrow, .curve, .curvedArrow],
        [.rectangle, .ellipse],
        [.pen, .cloud],
        [.highlight],
        [.text, .curvedText, .cloudText, .boxText, .ellipseText],
    ]

    /// The two ways of ringing a region, in the order they are always offered.
    ///
    /// Fixed so that the press which opens the slot and the tap which follows
    /// always land on the same thing.
    static let ringTools: [MarkupTool] = [.pen, .cloud]

    /// The tools sharing this one's place in the row, itself included.
    var slotMates: [MarkupTool] {
        MarkupTool.slots.first { $0.contains(self) } ?? [self]
    }

    /// The other tool sharing this one's place, if it shares one. What a long
    /// press swaps to when a slot holds exactly two.
    var alternate: MarkupTool? { slotMates.first { $0 != self } }

    /// What the ribbon puts in this tool's slot.
    ///
    /// The reader's pictures, tool for tool: a curved line on a picture and a
    /// curved line on a page are the same mark, so they are the same drawing.
    /// Two maps would be two that agreed until somebody changed one.
    var ribbonIcon: RibbonIcon {
        switch self {
        // A loose squiggle — deliberately the mark itself rather than a brush.
        case .pen: return .system("scribble.variable")
        // Horizontal, not diagonal.
        case .line: return .system("minus")
        case .arrow: return .system("arrow.right")
        case .rectangle: return .system("square")
        case .ellipse: return .system("circle")
        case .cloud: return .system("cloud")
        case .highlight: return .system("highlighter")
        case .text: return .system("textformat")
        case .curve: return .drawn(.curvedLine)
        case .curvedArrow: return .drawn(.curvedArrow)
        case .curvedText: return .drawn(.curvedText)
        case .cloudText: return .drawn(.cloudText)
        case .boxText: return .drawn(.boxText)
        case .ellipseText: return .drawn(.ellipseText)
        }
    }

    /// Build the shape a drag defines, for every tool but the traced ones.
    func shape(from start: CGPoint, to end: CGPoint) -> MarkupShape {
        switch self {
        case .line: return .line(from: start, to: end)
        case .arrow: return .arrow(from: start, to: end)
        case .rectangle: return .rectangle(PageRect(from: start, to: end))
        case .ellipse: return .ellipse(PageRect(from: start, to: end))
        case .highlight: return .highlight(PageRect(from: start, to: end))
        // These trace rather than drag; their shape comes from the whole stroke.
        case .pen, .cloud, .curve, .curvedArrow:
            return .freehand([start, end])
        // And these are not dragged at all: text is placed, then typed. The
        // canvas never asks this for one, and a shape built from two stray points
        // would be an empty mark somewhere nobody meant.
        case .text, .curvedText, .cloudText, .boxText, .ellipseText:
            return .freehand([])
        }
    }
}

/// The capture editor's ink.
///
/// Red first, and the default: markup on a picture of a page is almost always
/// pointing something out, and it has to hold up next to black text. The same six
/// constants the reader draws with, in a different order and with a different
/// first — the colours are shared, the palettes are not.
enum MarkupColors {
    static let palette: [MarkColor] = [
        AnnotationColors.red,
        AnnotationColors.blue,
        AnnotationColors.green,
        AnnotationColors.yellow,
        AnnotationColors.orange,
        AnnotationColors.pink,
    ]
}

/// Sizes and limits the capture editor works in. §D of the port audit.
enum MarkupMetrics {
    /// Default nib, in capture units. About a pen line on paper.
    static let strokePoints: CGFloat = 2.4

    /// Smallest mark worth committing, in capture units. A tap that moved a
    /// couple of points is somebody trying to scroll, and turning it into an
    /// invisible mark that has to be found and undone is worse than ignoring it.
    static let minimumMarkPoints: CGFloat = 4

    /// How long a finger must hold still, at the end of a stroke, to ask for a
    /// snap.
    static let dwellSeconds: TimeInterval = 0.3

    /// Movement below this does not cancel a dwell. A finger resting on glass
    /// still jitters by a point or so, and treating that as movement would mean
    /// the dwell never completed on a real hand.
    static let movementSlop: CGFloat = 1.5

    /// Too few points to be a shape; recognising three of them is guesswork.
    static let minimumStrokePoints = 6

    /// How near a finger has to be to grab words on a picture, in capture units.
    static let textGrabPoints: CGFloat = 14
}

// ------------------------------------------------------------------ the shapes --

/// Words written on a picture.
///
/// **No id, no page and no colour.** The colour belongs to the enclosing
/// `Markup`, exactly as it does for a stroke — one mark, one ink — and there is
/// nothing on a picture to give a mark an identity: the list is the drawing, in
/// the order it was made. The reader's `TextMark` carries all three because a
/// page caption is real text in a file that has to be found again; conflating the
/// two is what put ids and colours on the capture side and lost them on the
/// reader's.
struct MarkupTextShape: Equatable {
    var text: String
    /// The baseline, in capture units. One point for straight text.
    var path: [CGPoint]
    var font: PagifyFont
    var sizePoints: CGFloat
    var frame: TextFrame = .none
    /// How far the baseline turns from end to end, in degrees.
    var curveDegrees: CGFloat = 0
    /// How many lines the words are broken into.
    ///
    /// One means "however many newlines were typed", which is what a caption has
    /// always been. Anything more wraps the words to fill that many lines — a
    /// paragraph pasted in from a page has no newlines at all, and without this
    /// it became a single line of type running clean off the sheet.
    var lines: Int = 1

    var isCurved: Bool { path.count > 2 }
    var isMultiLine: Bool { text.contains("\n") || lines > 1 }
}

/// Marks drawn on a capture.
///
/// The split follows the same rule as the reader's: the wet stroke and the
/// preview drawing are the app's, the committed shape and the compositing are the
/// engine's. What leaves the app is drawn by the engine from the document and
/// these shapes, so nothing else can find its way into it.
enum MarkupShape {
    /// The stroke as drawn, unrecognised.
    case freehand([CGPoint])
    case line(from: CGPoint, to: CGPoint)
    /// A line with a head at `to`.
    case arrow(from: CGPoint, to: CGPoint)
    case rectangle(PageRect)
    case ellipse(PageRect)
    /// A translucent wash, for picking something out rather than ringing it.
    case highlight(PageRect)
    /// Words on the picture, kept as words all the way to the export and
    /// flattened to outlines only there. Storing the outlines instead would make
    /// the mark unreadable to everything that handles it — undo, hit tests, the
    /// live preview — and pin the letters to the size they were first drawn at.
    case text(MarkupTextShape)

    /// Whether a dragged shape is worth keeping.
    ///
    /// Same reasoning as the capture rectangle: a tap that moved a couple of
    /// points is someone trying to scroll.
    var isBigEnough: Bool {
        let floor = MarkupMetrics.minimumMarkPoints
        switch self {
        case .freehand(let points):
            return points.count > 1
        case .line(let from, let to), .arrow(let from, let to):
            return hypot(to.x - from.x, to.y - from.y) >= floor
        case .rectangle(let rect), .ellipse(let rect):
            // Either dimension: a long thin box is a legitimate mark.
            return (rect.right - rect.left) >= floor || (rect.bottom - rect.top) >= floor
        case .highlight(let rect):
            // Both, because a wash with no height washes nothing.
            return (rect.right - rect.left) >= floor && (rect.bottom - rect.top) >= floor
        case .text(let shape):
            // Blank words make no mark rather than an invisible one, which could
            // only be found by rubbing out at random.
            return !shape.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !shape.path.isEmpty
        }
    }
}

/// One committed mark, with how it is drawn.
struct Markup {
    var shape: MarkupShape
    var color: MarkColor
    /// Stroke width in capture units, so it keeps its weight at any export scale.
    /// Zero for the wash, whose strength is in the alpha instead.
    var widthPoints: CGFloat = MarkupMetrics.strokePoints
    /// Solid, dashed or dash-dot. Only the tools that draw a line offer anything
    /// but solid.
    var style: MarkupStyle = .solid
}

/// Build a mark from the tool that drew it and how heavy that tool is set to.
///
/// The highlighter's intensity rides in the colour's **alpha**, which is where the
/// engine reads it: a wash and a stroke are the same `Markup`, and giving the
/// highlighter a field of its own would mean every other tool carrying one it
/// ignores.
func markupFor(shape: MarkupShape, tool: MarkupTool, color: MarkColor,
               size: CGFloat, style: MarkupStyle = .solid) -> Markup {
    guard tool.isIntensity else {
        return Markup(shape: shape, color: color, widthPoints: size,
                      // Carried only by the tool that offers it, so a style
                      // chosen for lines does not quietly follow you to the box.
                      style: tool.hasLineStyle ? style : .solid)
    }

    // Truncating, not rounding: 0.5 is 127, which is what the Android test pins.
    // Not clamped at the top either — 1.0 must legitimately reach 255, and the
    // ceiling belongs to the engine. Non-finite is treated as nothing rather than
    // trapping in the conversion.
    let fraction = size.isFinite ? min(max(size, 0), 1) : 0
    let alpha = UInt32(fraction * 255)
    // The incoming alpha is masked away rather than multiplied: the palette is
    // opaque, and a stored alpha would compound every time the wash was restyled.
    let argb = (color.argb & 0x00FF_FFFF) | (alpha << 24)
    return Markup(shape: shape, color: MarkColor(argb: argb), widthPoints: 0)
}

// ------------------------------------------------------------ caption geometry --

// The same proportions `TextLayout.swift` lays a page caption out with. They are
// duplicated here only because that file keeps them private; W4's extraction of
// `textFrameBounds`/`textFrameOutline` as free functions is what folds the two
// copies back into one, and this block goes when it lands.
private let markupCapHeight: CGFloat = 0.72
private let markupDescender: CGFloat = 0.21
private let markupLineHeight: CGFloat = 1.25
private let markupEllipseReach: CGFloat = 1.47
private let markupEllipseSegments = 64
/// How thick a stroke the cloud's bumps are sized as, per point of type.
private let markupCloudBump: CGFloat = 0.17

extension MarkupTextShape {
    /// Every glyph of a caption, over however many lines it has.
    ///
    /// One line goes along the stored baseline, bent or straight. More than one is
    /// laid as a block: each line on its own baseline, one line height below the
    /// last, every line centred on the block — centred because a multi-line
    /// caption is usually inside a cloud or a box, and a frame drawn round
    /// ragged-left lines reads as a mistake.
    func layOutBlock() -> [GlyphPlacement] {
        let lines = captionLines(text, into: self.lines)
        if lines.count <= 1 {
            return layOutText(text, font: font, size: sizePoints, path: path)
        }
        guard let anchor = path.first else { return [] }

        let widest = lines.map { font.width(of: $0, size: sizePoints) }.max() ?? 0
        let leading = sizePoints * markupLineHeight

        return lines.enumerated().flatMap { index, line -> [GlyphPlacement] in
            let width = font.width(of: line, size: sizePoints)
            let start = CGPoint(x: anchor.x + (widest - width) / 2,
                                y: anchor.y + CGFloat(index) * leading)
            return layOutText(line, font: font, size: sizePoints,
                              path: straightBaseline(anchor: start, text: line,
                                                     font: font, size: sizePoints))
        }
    }

    /// The box the words occupy, before any margin.
    func blockBounds() -> PageRect {
        let anchor = path.first ?? .zero
        let lines = captionLines(text, into: self.lines)
        let widest = lines.map { font.width(of: $0, size: sizePoints) }.max() ?? 0
        let leading = sizePoints * markupLineHeight
        return PageRect(
            left: anchor.x,
            top: anchor.y - sizePoints * markupCapHeight,
            right: anchor.x + widest,
            bottom: anchor.y + sizePoints * markupDescender + CGFloat(lines.count - 1) * leading)
    }

    /// The box a frame is drawn on, the words plus their margin.
    func frameBounds() -> PageRect {
        let margin = sizePoints * cloudTextMarginFraction
        let box = blockBounds()
        return PageRect(left: box.left - margin, top: box.top - margin,
                        right: box.right + margin, bottom: box.bottom + margin)
    }

    /// The box the selection chrome is drawn on.
    ///
    /// Measured from the whole run on one line rather than from the block, which
    /// is what Android's selection does: the chrome says "these words are in
    /// hand", and it is drawn round the caption's own line even when the caption
    /// has since gained another.
    func runBounds() -> PageRect {
        let anchor = path.first ?? .zero
        let margin = sizePoints * cloudTextMarginFraction
        let runWidth = font.width(of: text, size: sizePoints)
        return PageRect(left: anchor.x - margin,
                        top: anchor.y - sizePoints * markupCapHeight - margin,
                        right: anchor.x + runWidth + margin,
                        bottom: anchor.y + sizePoints * markupDescender + margin)
    }

    /// The ring drawn around a framed caption, as one closed polyline.
    ///
    /// The cloud comes through the same `cloudOutline` the cloud tool uses, so a
    /// cloud round words and a cloud drawn by hand are the same notation. The
    /// ellipse is grown past the box rather than inscribed in it: an inscribed
    /// ellipse cuts every corner off the box, which on a line of text means
    /// clipping the first and last letters.
    func frameOutline() -> [CGPoint] {
        let box = frameBounds()
        let corners = [CGPoint(x: box.left, y: box.top),
                       CGPoint(x: box.right, y: box.top),
                       CGPoint(x: box.right, y: box.bottom),
                       CGPoint(x: box.left, y: box.bottom)]
        switch frame {
        case .none:
            return []
        case .cloud:
            return cloudOutline(corners, width: sizePoints * markupCloudBump)
        case .box:
            return corners + [corners[0]]
        case .ellipse:
            let cx = (box.left + box.right) / 2
            let cy = (box.top + box.bottom) / 2
            let rx = (box.right - box.left) / 2 * markupEllipseReach
            let ry = (box.bottom - box.top) / 2 * markupEllipseReach
            return (0...markupEllipseSegments).map { step in
                let angle = CGFloat(step) / CGFloat(markupEllipseSegments) * 2 * .pi
                return CGPoint(x: cx + cos(angle) * rx, y: cy + sin(angle) * ry)
            }
        }
    }

    /// True when `point` lands on these words, allowing `tolerance` either side.
    func isHitBy(_ point: CGPoint, tolerance: CGFloat) -> Bool {
        guard let anchor = path.first else { return false }

        if frame != .none || isMultiLine {
            // A block, or anything with a ring round it, is grabbable anywhere
            // over the words — which is what it looks like.
            let reach = tolerance + sizePoints * cloudTextMarginFraction
            return blockBounds().cgRect.insetBy(dx: -reach, dy: -reach).contains(point)
        }

        // The words run to the right of where they were placed, so the run has to
        // be measured out: testing the anchor alone makes a mark grabbable by its
        // first letter and nowhere else.
        let line = isCurved
            ? path
            : [anchor, CGPoint(x: anchor.x + font.width(of: text, size: sizePoints), y: anchor.y)]
        return isNear(point, polyline: line, within: tolerance + sizePoints)
    }

    /// The same words, shifted by `delta` capture units.
    func movedBy(_ delta: CGPoint) -> MarkupTextShape {
        var moved = self
        moved.path = path.map { CGPoint(x: $0.x + delta.x, y: $0.y + delta.y) }
        return moved
    }

    /// The same words, larger or smaller about where they were placed.
    ///
    /// Scaled about the anchor rather than about the middle, so the caption stays
    /// where it was put, and the line is scaled with it so a longer run does not
    /// lose its last letters — the layout walks the baseline and drops any glyph
    /// that runs off the end. The **achieved** factor is applied, not the
    /// requested one, so a caption already at the ceiling does not creep.
    func scaledBy(_ factor: CGFloat) -> MarkupTextShape {
        guard let anchor = path.first else { return self }
        let reached = min(max(sizePoints * factor, AnnotationMetrics.textRange.lowerBound),
                          AnnotationMetrics.textRange.upperBound)
        guard reached != sizePoints, sizePoints > 0 else { return self }

        let achieved = reached / sizePoints
        var scaled = self
        scaled.sizePoints = reached
        scaled.path = path.map {
            CGPoint(x: anchor.x + ($0.x - anchor.x) * achieved,
                    y: anchor.y + ($0.y - anchor.y) * achieved)
        }
        return scaled
    }

    /// The same caption, restyled.
    ///
    /// The baseline is rebuilt rather than adjusted, because everything about it
    /// follows from the other four: where it starts, what it says, how big, and
    /// how far it bends. Adjusting it in place is how a caption ends up in a face
    /// that no longer fits the line it sits on and loses its last letters.
    ///
    /// **No colour argument.** A capture caption has no colour of its own; the
    /// enclosing `Markup` carries it, and `setMarkupColor` therefore restyles
    /// nothing.
    func rebuilt(text: String? = nil, font: PagifyFont? = nil,
                 sizePoints: CGFloat? = nil, curveDegrees: CGFloat? = nil,
                 frame: TextFrame? = nil) -> MarkupTextShape {
        let words = text ?? self.text
        let face = font ?? self.font
        let requested = curveDegrees ?? self.curveDegrees
        let size = min(max(sizePoints ?? self.sizePoints,
                           AnnotationMetrics.textRange.lowerBound),
                       AnnotationMetrics.textRange.upperBound)

        // A caption that gained a second line straightens: stacked arcs curl into
        // each other and there is no answer for where the second one sits. The
        // requested bend is still stored, so losing the extra line brings it back.
        let bend = words.contains("\n") ? 0 : requested
        let anchor = path.first ?? .zero

        return MarkupTextShape(
            text: words,
            path: curvedBaseline(anchor: anchor,
                                 text: captionLines(words).first ?? "",
                                 font: face, size: size, degrees: bend),
            font: face,
            sizePoints: size,
            frame: frame ?? self.frame,
            curveDegrees: requested)
    }
}

// ----------------------------------------------------------------- the settings --

/// Everything the capture editor's tools are currently set to.
///
/// Lives and dies with the picture. None of it is written to the document — a
/// capture's marks are drawn into an exported image by the engine and never touch
/// the PDF — which is why there is no history here beyond dropping the last mark.
struct MarkupSettings {
    var tool: MarkupTool = .pen

    /// Whether that tool is actually held.
    ///
    /// Separate from `tool` rather than making it optional, so putting a tool
    /// down and picking it back up returns the one you had, with its colour and
    /// its weight, rather than starting again at the pen. This is the capture
    /// side's answer to the reader's `none` sentinel, and the reason this enum
    /// does not have one.
    var armed: Bool = true

    /// One colour for every tool. Red by default.
    var color: MarkColor = AnnotationColors.red

    /// How heavy each tool draws — nib width for most, intensity for the wash.
    ///
    /// Per tool rather than shared: somebody who has set a fine pen does not
    /// expect picking up the highlighter and putting it down again to have
    /// changed it.
    var sizes: [MarkupTool: CGFloat] = MarkupTool.defaultSizes()

    var style: MarkupStyle = .solid

    /// The face words are written in, and how big. Shared with the reader's text
    /// tools in Android's state; held here because this editor outlives nothing.
    var font: PagifyFont = .helvetica
    var textSize: CGFloat = 12
    /// Straight by default — a caption that bends is the exception.
    var curveDegrees: CGFloat = 0

    /// The caption the ribbon is editing, **by position in the mark list**.
    ///
    /// Deliberately not an id: a capture mark has none, nothing reorders the
    /// list, and the index a drag started on is the mark it started on.
    var selectedIndex: Int?

    /// How heavy the tool in hand is set.
    var size: CGFloat { sizes[tool] ?? tool.defaultSize }

    /// Pick a tool up. Choosing one is also how you arm it.
    ///
    /// The colour is **not** snapped to a palette here, unlike the reader's:
    /// there is only one capture palette, so there is nothing to snap between.
    mutating func select(_ next: MarkupTool) {
        tool = next
        armed = true
    }

    /// Put the tool down, so a stray finger cannot draw. Which tool it was is
    /// remembered.
    mutating func disarm() { armed = false }

    /// Set how heavy a tool draws, against the tool it belongs to.
    mutating func setSize(_ value: CGFloat, for tool: MarkupTool) {
        let range = tool.sizeRange
        sizes[tool] = min(max(value, range.lowerBound), range.upperBound)
    }

    mutating func setTextSize(_ value: CGFloat) {
        textSize = min(max(value, AnnotationMetrics.textRange.lowerBound),
                       AnnotationMetrics.textRange.upperBound)
    }
}

// -------------------------------------------------------------------- gestures --

/// One drag on a capture, from touch-down to lift.
///
/// A plain object rather than logic inside the touch handler, because the rules
/// here are the ones worth testing: when a stroke snaps, when it stays as drawn,
/// and — the invariant the design names explicitly — that **nothing calls the
/// engine while a finger is down.** `up()` is the only method that returns
/// anything to act on.
///
/// Everything is in capture units; the caller converts before it gets here.
final class MarkupGesture {
    /// What to do about a lift.
    enum Outcome {
        /// Too small to be meant, or nothing was drawn.
        case nothing
        /// Ready to add, with no engine involved.
        ///
        /// Several shapes, because one gesture is not always one mark: a curved
        /// arrow is a curve and two barbs, kept apart so its tip stays sharp.
        case commit([MarkupShape])
        /// Hand these points to the recogniser, then commit whatever comes back.
        /// Returned rather than recognised here so the engine call happens after
        /// the finger is up, on the caller's terms.
        case recognise([CGPoint])
    }

    private let tool: MarkupTool
    /// How heavy the tool is set, in capture units.
    ///
    /// Only the cloud reads it, and only to size its scallops. It is held from
    /// construction because the preview needs the same number: a cloud previewed
    /// with one bump size and committed with another would change shape as the
    /// finger left the glass.
    private let sizePoints: CGFloat

    private var points: [CGPoint] = []

    /// True once the finger has held still long enough to mean it.
    private(set) var isDwelling = false

    init(tool: MarkupTool, sizePoints: CGFloat = MarkupMetrics.strokePoints) {
        self.tool = tool
        self.sizePoints = sizePoints
    }

    /// What to draw while the finger is down; empty before it lands.
    ///
    /// A list because one gesture is not always one shape. Built by the same code
    /// that will commit it, so what is under the finger is what is released —
    /// showing a raw trace and swapping it for scallops on lift means aiming at
    /// something you cannot see.
    var preview: [MarkupShape] {
        guard points.count >= 2 else { return [] }
        if tool.isDragged {
            return [tool.shape(from: points[0], to: points[points.count - 1])]
        }
        if tool.correctsOnRelease {
            return [.freehand(points)]
        }
        return tracedShapes(points)
    }

    func down(at point: CGPoint) {
        points = [point]
        isDwelling = false
    }

    func move(to point: CGPoint) {
        // Any real movement cancels a dwell: the hold has to be the *last* thing
        // that happened, or a pause halfway through a long squiggle would snap it.
        if let last = points.last {
            if hypot(point.x - last.x, point.y - last.y) > MarkupMetrics.movementSlop {
                isDwelling = false
            }
        } else {
            isDwelling = false
        }
        points.append(point)
    }

    /// The finger has been still for the dwell. Only meaningful for the pen.
    func still() {
        if tool.recognises, points.count >= MarkupMetrics.minimumStrokePoints {
            isDwelling = true
        }
    }

    /// Lift, and what to do about it.
    ///
    /// The dwell decides whether recognition is even attempted. That is the whole
    /// anti-surprise rule: a stroke drawn and lifted straight away is kept exactly
    /// as drawn, and only a deliberate hold at the end asks for a shape.
    func up() -> Outcome {
        let stroke = points
        points = []
        let dwelled = isDwelling
        isDwelling = false

        guard stroke.count >= 2 else { return .nothing }

        if tool.isDragged {
            let shape = tool.shape(from: stroke[0], to: stroke[stroke.count - 1])
            return shape.isBigEnough ? .commit([shape]) : .nothing
        }
        // Held still at the end: ask the engine what this is.
        if dwelled { return .recognise(stroke) }
        // Traced. Nothing is asked of the engine — a cloud and a curve are
        // already the shapes they mean.
        let shapes = tracedShapes(stroke)
        return shapes.isEmpty ? .nothing : .commit(shapes)
    }

    /// Abandoned — another pointer arrived, or the editor closed.
    func cancel() {
        points = []
        isDwelling = false
    }

    /// What a traced stroke becomes, for whichever tool traced it.
    private func tracedShapes(_ stroke: [CGPoint]) -> [MarkupShape] {
        let shapes: [MarkupShape]
        switch tool {
        case .cloud:
            shapes = [.freehand(cloudOutline(stroke, width: sizePoints))]
        case .curve:
            shapes = [.freehand(curveThrough(stroke))]
        case .curvedArrow:
            // Solid on purpose: the line type is applied when the mark is drawn,
            // and dashing here would bake it into the committed geometry.
            shapes = curvedArrowStrokes(stroke, width: sizePoints, style: .solid)
                .map { MarkupShape.freehand($0) }
        default:
            shapes = [.freehand(stroke)]
        }
        return shapes.filter(\.isBigEnough)
    }
}

// ------------------------------------------------------------------- the wire --

extension Markup {
    /// The engine's form.
    ///
    /// Both sides pin this shape in a test with a literal string rather than
    /// sharing a builder, because a shared builder lets the two agree on the
    /// wrong thing.
    var wireJSON: [String: Any] {
        ["shape": shape.wireJSON,
         "color": color.json,
         "widthPt": widthPoints,
         "style": style.wireName]
    }

    /// One mark as the marks the engine draws.
    ///
    /// Nearly always itself. Framed text is two: the ring is a stroked line and
    /// the letters are a filled shape, and no single drawing operation is both.
    /// They stay one mark everywhere else — one undo, one thing to move — and
    /// come apart only here, on the way out.
    func forWire() -> [Markup] {
        guard case .text(let caption) = shape, caption.frame != .none else { return [self] }
        let ring = caption.frameOutline()
        guard ring.count >= 2 else { return [self] }

        var outline = self
        outline.shape = .freehand(ring)
        outline.widthPoints = caption.sizePoints * textFrameStroke
        outline.style = .solid
        // The ring first, so the letters are drawn over it where they meet.
        return [outline, self]
    }
}

extension MarkupStyle {
    /// What the engine calls this line type. The enum's own raw values already
    /// match; this exists so the name the wire uses is stated once.
    var wireName: String { rawValue }
}

extension MarkupShape {
    var wireJSON: [String: Any] {
        switch self {
        case .freehand(let points):
            return ["kind": "freehand", "points": points.map(pointJSON)]
        case .line(let from, let to):
            return ["kind": "line", "from": pointJSON(from), "to": pointJSON(to)]
        case .arrow(let from, let to):
            return ["kind": "arrow", "from": pointJSON(from), "to": pointJSON(to)]
        case .rectangle(let rect):
            return ["kind": "rect", "rect": rect.json]
        case .ellipse(let rect):
            return ["kind": "ellipse", "rect": rect.json]
        case .highlight(let rect):
            return ["kind": "highlight", "rect": rect.json]
        case .text(let caption):
            // A picture has no text layer, so words on one leave as the outlines
            // of their letters. Never sent as itself — see `forWire`.
            return ["kind": "glyphs",
                    "contours": caption.glyphContours().map { $0.map(pointJSON) }]
        }
    }
}

private func pointJSON(_ point: CGPoint) -> [String: Any] {
    ["x": point.x, "y": point.y]
}

/// Every mark on the picture, as the engine's array — with framed captions taken
/// apart into their ring and their letters.
func markupWireJSON(_ marks: [Markup]) -> String {
    let body = marks.flatMap { $0.forWire() }.map(\.wireJSON)
    guard let data = try? JSONSerialization.data(withJSONObject: body),
          let text = String(data: data, encoding: .utf8) else {
        return "[]"
    }
    return text
}

/// The points of a stroke, for the recogniser and for a lasso mask.
func strokeWireJSON(_ points: [CGPoint]) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: points.map(pointJSON)),
          let text = String(data: data, encoding: .utf8) else {
        return "[]"
    }
    return text
}

/// Read back what the recogniser made of a stroke.
///
/// An unrecognised kind comes back as freehand rather than throwing: a shape the
/// app does not know is still a mark someone drew, and losing it would be worse
/// than drawing it plainly.
func markupShape(fromWireJSON json: String, fallback: [CGPoint]) -> MarkupShape {
    guard let data = json.data(using: .utf8),
          let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
        return .freehand(fallback)
    }

    func point(_ key: String) -> CGPoint {
        guard let o = root[key] as? [String: Any] else { return .zero }
        return CGPoint(x: o["x"] as? CGFloat ?? 0, y: o["y"] as? CGFloat ?? 0)
    }
    func rect(_ key: String) -> PageRect {
        guard let o = root[key] as? [String: Any] else {
            return PageRect(left: 0, top: 0, right: 0, bottom: 0)
        }
        return PageRect(left: o["left"] as? CGFloat ?? 0, top: o["top"] as? CGFloat ?? 0,
                        right: o["right"] as? CGFloat ?? 0, bottom: o["bottom"] as? CGFloat ?? 0)
    }

    switch root["kind"] as? String {
    case "line": return .line(from: point("from"), to: point("to"))
    case "arrow": return .arrow(from: point("from"), to: point("to"))
    case "rect": return .rectangle(rect("rect"))
    case "ellipse": return .ellipse(rect("rect"))
    case "highlight": return .highlight(rect("rect"))
    case "freehand":
        guard let points = root["points"] as? [[String: Any]] else { return .freehand(fallback) }
        return .freehand(points.map {
            CGPoint(x: $0["x"] as? CGFloat ?? 0, y: $0["y"] as? CGFloat ?? 0)
        })
    default:
        return .freehand(fallback)
    }
}

/// What the engine makes of a drawn stroke.
///
/// Pure geometry, so unlike everything else at this boundary it takes no document
/// and no lock. It is still called after the lift rather than during the drag, so
/// nothing can cost a frame mid-stroke.
func recogniseMarkupStroke(_ points: [CGPoint]) -> MarkupShape {
    guard let json = PagifyEngine.string(pagify_recognise_stroke(strokeWireJSON(points))) else {
        return .freehand(points)
    }
    return markupShape(fromWireJSON: json, fallback: points)
}

// ------------------------------------------------------------ letters as shapes --

#if canImport(UIKit)
extension MarkupTextShape {
    /// Letters as the outlines they are made of.
    ///
    /// A capture is a picture. It has no text layer, so words drawn on one cannot
    /// be written as text the way they are on a page — they have to become
    /// shapes. Core Text knows the shapes, since it is what drew them on screen,
    /// and this walks the result into the plain polylines the engine fills.
    ///
    /// Sampled rather than sent as curves because the wire carries points. The
    /// step is a fraction of the point size, so a letter is sampled as finely at
    /// nine points as at seventy-two.
    func glyphContours() -> [[CGPoint]] {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return [] }

        let uiFont = font.uiFont(size: sizePoints)
        let letters = CGMutablePath()

        for placement in layOutBlock() {
            guard let glyph = outline(of: placement, in: uiFont) else { continue }
            // Rotated about its own origin and then carried to it, which is the
            // order that puts a letter on a curve. The other way round rotates
            // the whole line about the picture's corner and scatters the word
            // across it.
            let place = CGAffineTransform(rotationAngle: placement.radians)
                .concatenating(CGAffineTransform(translationX: placement.origin.x,
                                                 y: placement.origin.y))
            letters.addPath(glyph, transform: place)
        }

        let step = max(sizePoints * glyphSampleFraction, glyphMinimumStep)
        return sampledContours(of: letters, step: step)
    }

    /// One character's outline, at the origin, in the app's y-downward space.
    ///
    /// Core Text hands back glyph paths with y increasing **upwards**, which is
    /// the one convention nothing else here uses; left unflipped every caption
    /// comes out mirrored about its own baseline.
    private func outline(of placement: GlyphPlacement, in uiFont: UIFont) -> CGPath? {
        // By glyph id where the shaper gave one. Re-shaping the cluster's
        // characters here would ask Core Text to lay out one Arabic letter with
        // no neighbours, and it can only answer with the isolated form — the
        // outline would then disagree with both the drawn caption and the file.
        if placement.id != 0, font.asset != nil {
            var flip = CGAffineTransform(scaleX: 1, y: -1)
            return CTFontCreatePathForGlyph(uiFont as CTFont,
                                            CGGlyph(truncatingIfNeeded: placement.id),
                                            &flip)
        }

        let character = placement.text
        guard !character.isEmpty else { return nil }
        let attributed = NSAttributedString(string: character, attributes: [.font: uiFont])
        let line = CTLineCreateWithAttributedString(attributed)
        guard let runs = CTLineGetGlyphRuns(line) as? [CTRun] else { return nil }

        let flip = CGAffineTransform(scaleX: 1, y: -1)
        let combined = CGMutablePath()
        var wrote = false

        for run in runs {
            let count = CTRunGetGlyphCount(run)
            guard count > 0 else { continue }
            var glyphs = [CGGlyph](repeating: 0, count: count)
            var positions = [CGPoint](repeating: .zero, count: count)
            CTRunGetGlyphs(run, CFRangeMake(0, count), &glyphs)
            CTRunGetPositions(run, CFRangeMake(0, count), &positions)

            // The run's own face, not the one asked for: Core Text substitutes
            // when the requested face has no glyph, and drawing the substitute's
            // glyph ids through the original font would emit whatever letters
            // happened to share those numbers.
            let attributes = CTRunGetAttributes(run) as NSDictionary
            guard let runFont = attributes[kCTFontAttributeName as String] else { continue }
            let face = runFont as! CTFont

            for index in 0..<count {
                guard let path = CTFontCreatePathForGlyph(face, glyphs[index], nil) else { continue }
                let placed = CGAffineTransform(translationX: positions[index].x,
                                               y: positions[index].y).concatenating(flip)
                combined.addPath(path, transform: placed)
                wrote = true
            }
        }
        return wrote ? combined : nil
    }
}

/// Every contour of a path, sampled into points at most `step` apart.
///
/// Contour by contour rather than over the path as a whole: the hole in an "o" is
/// a contour of its own, and running the two together joins them with a line
/// straight through the letter.
private func sampledContours(of path: CGPath, step: CGFloat) -> [[CGPoint]] {
    var contours: [[CGPoint]] = []
    var current: [CGPoint] = []
    var cursor = CGPoint.zero
    var start = CGPoint.zero

    func finish() {
        if current.count >= 3 { contours.append(resampled(current, step: step)) }
        current = []
    }

    path.applyWithBlock { element in
        let points = element.pointee.points
        switch element.pointee.type {
        case .moveToPoint:
            finish()
            cursor = points[0]
            start = cursor
            current = [cursor]
        case .addLineToPoint:
            current.append(points[0])
            cursor = points[0]
        case .addQuadCurveToPoint:
            current += quadPoints(from: cursor, control: points[0], to: points[1], step: step)
            cursor = points[1]
        case .addCurveToPoint:
            current += cubicPoints(from: cursor, c1: points[0], c2: points[1],
                                   to: points[2], step: step)
            cursor = points[2]
        case .closeSubpath:
            if cursor != start { current.append(start) }
            finish()
            cursor = start
        @unknown default:
            break
        }
    }
    finish()
    return contours
}

/// The samples of one curve, excluding its start — the previous element already
/// left that point in the list, and repeating it puts a doubled point on every
/// join.
private func quadPoints(from: CGPoint, control: CGPoint, to: CGPoint,
                        step: CGFloat) -> [CGPoint] {
    let steps = curveSteps(hypot(control.x - from.x, control.y - from.y)
                           + hypot(to.x - control.x, to.y - control.y), step: step)
    return (1...steps).map { index in
        let t = CGFloat(index) / CGFloat(steps)
        let u = 1 - t
        return CGPoint(x: u * u * from.x + 2 * u * t * control.x + t * t * to.x,
                       y: u * u * from.y + 2 * u * t * control.y + t * t * to.y)
    }
}

private func cubicPoints(from: CGPoint, c1: CGPoint, c2: CGPoint, to: CGPoint,
                         step: CGFloat) -> [CGPoint] {
    let rough = hypot(c1.x - from.x, c1.y - from.y)
        + hypot(c2.x - c1.x, c2.y - c1.y)
        + hypot(to.x - c2.x, to.y - c2.y)
    let steps = curveSteps(rough, step: step)
    return (1...steps).map { index in
        let t = CGFloat(index) / CGFloat(steps)
        let u = 1 - t
        return CGPoint(
            x: u * u * u * from.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t * t * t * to.x,
            y: u * u * u * from.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * to.y)
    }
}

private func curveSteps(_ roughLength: CGFloat, step: CGFloat) -> Int {
    guard roughLength.isFinite, step > 0 else { return 2 }
    return min(max(Int(roughLength / step), 2), 64)
}

/// A contour re-spaced evenly along its own length.
///
/// Evenly by arc length rather than by control point, so a letter's straight
/// stems do not come back as two points while its bowls come back as forty. The
/// ceiling is not a quality setting — nothing real approaches it — it is there so
/// a pathological glyph cannot put a megabyte of coordinates on the wire.
private func resampled(_ contour: [CGPoint], step: CGFloat) -> [CGPoint] {
    let lengths = zip(contour, contour.dropFirst())
        .map { hypot($0.1.x - $0.0.x, $0.1.y - $0.0.y) }
    let length = lengths.reduce(0, +)
    guard length > 0, step > 0, !lengths.isEmpty else { return contour }

    let count = min(max(Int(length / step), 3), glyphMaximumSamples)
    var out: [CGPoint] = []
    out.reserveCapacity(count)

    // One forward walk for the whole contour rather than a search per sample:
    // the targets only ever increase, so the cursor never has to go back.
    var index = 0
    var travelled: CGFloat = 0
    for sample in 0..<count {
        let target = length * CGFloat(sample) / CGFloat(count)
        while index < lengths.count - 1, travelled + lengths[index] < target {
            travelled += lengths[index]
            index += 1
        }
        let segment = lengths[index]
        let along = segment > 0 ? min(max((target - travelled) / segment, 0), 1) : 0
        let from = contour[index]
        let to = contour[index + 1]
        out.append(CGPoint(x: from.x + (to.x - from.x) * along,
                           y: from.y + (to.y - from.y) * along))
    }
    return out
}

/// How finely a letter is sampled, as a fraction of its point size.
private let glyphSampleFraction: CGFloat = 0.03
/// Below this the sampling stops buying anything a viewer can see.
private let glyphMinimumStep: CGFloat = 0.05
private let glyphMaximumSamples = 512
#endif
