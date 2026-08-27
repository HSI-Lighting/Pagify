import CoreGraphics
import Foundation

/// What the **reader** can mark a page with. Android's `AnnotationTool` in
/// `core/Annotation.kt`.
///
/// Nineteen cases, in Android's declaration order — the order is load-bearing,
/// because anything that persists or transmits an ordinal maps to a different
/// tool if it drifts.
///
/// Deliberately **not** the capture editor's tool set. The reader marks a page
/// and the screenshot editor marks a picture; they can do different things, so
/// Android keeps two enums and so does this. `MarkupTool` is reserved for that
/// second one — see the capture editor when it is built.
enum AnnotationTool: String, CaseIterable, Identifiable {
    case none
    case highlight
    case pen
    case line
    case arrow
    case rectangle
    case ellipse
    case cloud
    case curve
    case curvedArrow
    case text
    case curvedText
    case cloudText
    case boxText
    case ellipseText
    case note
    case signature
    case eraser
    case snapshot

    var id: String { rawValue }

    /// The names on the ribbon. Shared word-for-word with the capture editor for
    /// the twelve tools both have — one mark, one name.
    var label: String {
        switch self {
        case .none: return "None"
        case .highlight: return "Highlight"
        case .pen: return "Freehand"
        case .line: return "Line"
        case .arrow: return "Arrow"
        case .rectangle: return "Box"
        case .ellipse: return "Circle"
        case .cloud: return "Cloud"
        case .curve: return "Curved line"
        case .curvedArrow: return "Curved arrow"
        case .text: return "Text"
        case .curvedText: return "Curved text"
        case .cloudText: return "Clouded text"
        case .boxText: return "Boxed text"
        case .ellipseText: return "Circled text"
        case .note: return "Note"
        case .signature: return "Signature"
        case .eraser: return "Eraser"
        case .snapshot: return "Snapshot"
        }
    }

    /// The tools that draw, in the order the palette offers them.
    ///
    /// The pen first because it is the one most reached for, and the shapes after
    /// it in the order a drawing needs them.
    static let drawingTools: [AnnotationTool] = [
        .pen, .line, .arrow, .curve, .curvedArrow, .rectangle, .ellipse, .cloud,
        .text, .curvedText, .cloudText, .boxText, .ellipseText,
    ]

    /// Whether this tool draws ink, and so takes a size, a colour and a line type.
    var draws: Bool { AnnotationTool.drawingTools.contains(self) }

    /// Whether this tool makes a mark, and so takes settings of its own.
    ///
    /// The highlighter is one: it has a colour, even though it has neither a nib
    /// width nor a line type.
    var marks: Bool { draws || self == .highlight }

    /// Whether this tool follows the finger rather than taking two corners.
    ///
    /// Exactly four. A signature is **not** here: it is captured in its own sheet
    /// with its own nib and its own bounds, not traced onto the page.
    var tracesPath: Bool {
        switch self {
        case .pen, .cloud, .curve, .curvedArrow: return true
        default: return false
        }
    }

    /// Whether this tool builds its mark from two corners rather than tracing.
    ///
    /// **This is true for the five text tools too** — a caption is dragged to give
    /// it a baseline, not tapped. Reading `isDragged` as "is a shape" is a mistake
    /// the port made once already.
    var isDragged: Bool { draws && !tracesPath }

    /// Whether the traced path is replaced on release by the shape it was meant to
    /// be — the curves, which straighten into one smooth bend.
    var correctsOnRelease: Bool { self == .curve || self == .curvedArrow }

    /// Whether this tool writes words rather than drawing.
    ///
    /// It takes the same band as the drawing tools, but two of the slots mean
    /// something else: the weight becomes a font size and the line type becomes
    /// the font itself. Neither a nib width nor a dash means anything to a letter.
    var writesText: Bool {
        switch self {
        case .text, .curvedText, .cloudText, .boxText, .ellipseText: return true
        default: return false
        }
    }

    /// Whether this tool writes on a bent line. The bend is a setting rather than
    /// something drawn: a short caption covers only the first part of a long
    /// stroke, and the first part of any hand-drawn arc is its straightest.
    var bendsText: Bool { self == .curvedText }

    /// What this tool draws around the words it writes, if anything. The frame
    /// belongs to the tool rather than to a separate setting: picking "clouded
    /// text" is picking the cloud.
    var textFrame: TextFrame {
        switch self {
        case .cloudText: return .cloud
        case .boxText: return .box
        case .ellipseText: return .ellipse
        default: return .none
        }
    }

    /// Whether the slot's glyph previews this one. A group can hold more than a
    /// slot can show; nothing is hidden by it, because the picker always lists the
    /// whole group.
    var inPreview: Bool { self != .boxText && self != .ellipseText }

    /// The drawing tools, in the slots the ribbon gives them.
    ///
    /// Grouped by what the mark *is*, not by how it is made: a line and an arrow
    /// are one question, and so are the ways of going round something.
    static let drawingGroups: [[AnnotationTool]] = [
        [.line, .arrow, .curve, .curvedArrow],
        [.rectangle, .ellipse],
        [.pen, .cloud],
        [.text, .curvedText, .cloudText, .boxText, .ellipseText],
    ]

    /// The tools sharing this one's slot, itself included, or just itself.
    var slotMates: [AnnotationTool] {
        AnnotationTool.drawingGroups.first { $0.contains(self) } ?? [self]
    }
}

/// Everything the reader's pen is currently set to. Android's annotation half of
/// `PdfReaderState`.
struct AnnotationSettings: Equatable {
    var tool: AnnotationTool = .none

    /// **One** colour, not two. Android keeps a single `penColor` and snaps it to
    /// whichever palette the armed tool uses — holding a second "highlight
    /// colour" alongside it puts the same question on screen twice and lets the
    /// two disagree.
    var penColor: MarkColor = AnnotationColors.yellow

    /// Nib width in page points, so a mark scales with the page rather than with
    /// the zoom it happened to be drawn at.
    var strokeWidth: CGFloat = 2.0
    var style: MarkupStyle = .solid

    var font: PagifyFont = .helvetica
    /// Point size, as a printer means it.
    var textSize: CGFloat = 12
    /// The largest size that still fits the page, recomputed as pages change.
    /// The last colour each family was set to. A custom colour off the wheel is
    /// remembered the same way a palette one is.
    /// How far the caption in hand is turned, in degrees.
    ///
    /// Kept beside the ribbon's other values so the control has something to bind
    /// to, and pulled from the caption whenever one is taken in hand — the mark is
    /// the authority, this is only what the slider is showing.
    var textTurnDegrees: CGFloat = 0
    var heldHighlightColor: MarkColor = AnnotationColors.highlightPalette[0]
    var heldMarkColor: MarkColor = AnnotationColors.markerPalette[0]

    var textSizeCeiling: CGFloat = AnnotationMetrics.textRange.upperBound
    /// How far the baseline turns from end to end, in degrees. Straight by
    /// default — a caption that bends is the exception.
    var curveDegrees: CGFloat = 0
    /// False once a caption has more than one line: a block does not bend, and the
    /// ribbon stops offering it.
    var textBendApplies: Bool = true

    /// The caption currently in hand, addressed **by id** — not by position in any
    /// list, which changes under it the moment anything else is added or erased.
    var selectedTextId: Int64?

    /// Whether the snapshot tool draws around a region rather than boxing it.
    var captureLasso: Bool = false

    /// The palette the armed tool draws from.
    var palette: [MarkColor] {
        tool == .highlight ? AnnotationColors.highlightPalette : AnnotationColors.markerPalette
    }

    /// Arm a tool, keeping the colour if the new tool's palette has it.
    ///
    /// Without the snap, picking up the highlighter after drawing in red asks for
    /// a red wash — a colour its palette does not offer and which reads as a
    /// mistake over text.
    mutating func select(_ next: AnnotationTool) {
        // What each family was last set to, so switching between them is not the
        // same as forgetting.
        //
        // Android snaps to the palette's first colour whenever the chosen one is
        // not in it, and its toolbar toggles an armed tool to `.none` — so putting
        // the highlighter down asked for the *marker* palette, threw the highlight
        // colour away, and picking the highlighter up again landed on yellow. The
        // colour survived only while the panel was open. Android does this too;
        // it is wrong in both.
        //
        // The reason behind the snap is kept: a highlight wash has to read behind
        // text and ink has to read on white, so a colour chosen for one is wrong
        // in the other. Only the forgetting goes.
        if tool == .highlight { heldHighlightColor = penColor }
        else if tool != .none { heldMarkColor = penColor }

        let wasHighlight = tool == .highlight
        tool = next

        // Putting a tool **down** switches to no family at all, so there is
        // nothing to conform to and nothing to repaint.
        if next != .none, next == .highlight ? !wasHighlight : wasHighlight {
            penColor = next == .highlight ? heldHighlightColor : heldMarkColor
        }
        // A caption stays selected only while something that could restyle it is
        // held. Anything else, and the ribbon would be editing a mark nobody can
        // see is chosen.
        if !next.writesText {
            selectedTextId = nil
            // The derived pair goes with it: a ceiling left behind from the
            // caption last held would cap the *next* one against the wrong page.
            textSizeCeiling = AnnotationMetrics.textRange.upperBound
            textBendApplies = true
        }
    }

    mutating func setStrokeWidth(_ width: CGFloat) {
        strokeWidth = min(max(width, AnnotationMetrics.strokeClamp.lowerBound),
                          AnnotationMetrics.strokeClamp.upperBound)
    }

    mutating func setTextSize(_ size: CGFloat) {
        textSize = min(max(size, AnnotationMetrics.textRange.lowerBound), textSizeCeiling)
    }
}
