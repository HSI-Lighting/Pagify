import CoreGraphics
import Foundation

/// The reader's palette. Android's `core/AnnotationColors.kt`.
///
/// **Every constant here is opaque.** The highlighter's wash is applied at draw
/// time by `highlightAlpha`, not by storing a half-transparent colour — a stored
/// alpha would be written into the PDF and the mark would come back paler every
/// time it was re-saved.
enum AnnotationColors {
    static let yellow = MarkColor(argb: 0xFF_FF_E0_66)
    static let green = MarkColor(argb: 0xFF_8C_E9_9A)
    static let blue = MarkColor(argb: 0xFF_74_C0_FC)
    static let pink = MarkColor(argb: 0xFF_FF_A8_C5)
    static let orange = MarkColor(argb: 0xFF_FF_C0_78)
    static let red = MarkColor(argb: 0xFF_FF_6B_6B)

    /// What the highlighter offers.
    static let highlightPalette: [MarkColor] = [yellow, green, blue, pink, orange, red]

    /// What everything else offers. A separate list on purpose: a marker line is
    /// read at full strength and wants saturated ink, where a highlight is read
    /// *through* and wants a pastel.
    static let markerPalette: [MarkColor] = [
        MarkColor(argb: 0xFF_E0_31_31),
        MarkColor(argb: 0xFF_19_71_C2),
        MarkColor(argb: 0xFF_2F_9E_44),
        MarkColor(argb: 0xFF_F0_8C_00),
        MarkColor(argb: 0xFF_21_25_29),
        MarkColor(argb: 0xFF_9C_36_B5),
    ]

    /// How much of the page shows through a highlight, applied when it is drawn
    /// and when it is written — never baked into the stored colour.
    static let highlightAlpha: CGFloat = 0.35
}

/// Sizes and limits the reader works in. §D of the port audit.
enum AnnotationMetrics {
    /// `ANNOTATION_STROKE_WIDTHS`.
    static let strokePresets: [CGFloat] = [1.2, 2.4, 5]
    /// What the slider spans.
    static let strokeSlider: ClosedRange<CGFloat> = 0.6...16
    /// What `setStrokeWidth` clamps to.
    ///
    /// Deliberately wider than the slider — that inconsistency is in the Android
    /// source and is ported rather than tidied, because a width arriving from a
    /// restored mark or a future control must not be silently pulled to 16.
    static let strokeClamp: ClosedRange<CGFloat> = 0.5...24

    /// `TEXT_SIZES`.
    static let textPresets: [CGFloat] = [9, 12, 18]
    static let textRange: ClosedRange<CGFloat> = 6...400
    /// How much of the page a caption may fill, which is what caps the size.
    static let textPageFraction: CGFloat = 0.94

    /// How far a finger may travel and still be a tap, in screen points.
    ///
    /// A tap is a distance, never an event count: a still finger on a simulator
    /// sends two events, and a real one resting on glass sends a dozen. Counting
    /// them made a tap on a caption place a second caption on top of it on any
    /// hand that is not perfectly steady.
    static let tapSlop: CGFloat = 10

    /// The nib a signature is written at. Not the reader's pen width — a
    /// signature is not a pen line whose weight anyone chose.
    static let signatureWidth: CGFloat = 2

    /// How far a note's marker reaches from its anchor.
    static let noteMarkerRadius: CGFloat = 7

    static let curvePresets: [CGFloat] = [-60, 0, 60]
    static let curveLimit: CGFloat = 180
}
