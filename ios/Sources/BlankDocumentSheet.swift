import SwiftUI

/// Everything the reader chose about the paper.
struct BlankSheet {
    let count: Int
    let size: SheetSize
    /// The paper's colour, or nil for paper left the colour paper already is.
    let fill: MarkColor?
    let ruling: Ruling
    /// Only meaningful when a whole document is being made.
    let name: String
}

/// One offered sheet size — a label and the points it measures.
///
/// A value rather than a fixed set of cases, because the most useful option is
/// the one no enumeration can name: the size of the page the new sheet will
/// follow.
struct SheetSize: Equatable, Identifiable {
    let label: String
    let size: CGSize

    var id: String { "\(label) \(size.width)x\(size.height)" }

    /// Within a point: paper sizes are quoted in millimetres and converted.
    func matches(_ other: CGSize) -> Bool {
        abs(size.width - other.width) < 1 && abs(size.height - other.height) < 1
    }

    /// Wide, never tall. Applied only on the landscape branch, so choosing
    /// Portrait leaves a page that was already wider than it is tall alone.
    func turnedOnItsSide() -> SheetSize {
        size.width >= size.height
            ? self
            : SheetSize(label: label, size: CGSize(width: size.height, height: size.width))
    }
}

/// The name `ReaderModel.createBlank` and the library's `+` still speak in.
typealias PaperSize = SheetSize

/// What is printed on the sheet before anything is written on it. The codes
/// match the engine's `Ruling::from_code`.
enum Ruling: Int, CaseIterable, Identifiable {
    case none = 0, lined = 1, grid = 2, dots = 3

    var id: Int { rawValue }

    var label: String {
        switch self {
        case .none: return "Plain"
        case .lined: return "Lined"
        case .grid: return "Grid"
        case .dots: return "Dots"
        }
    }
}

/// A sheet of paper: how many, how big, which way up, what colour, and what is
/// already printed on it.
///
/// Asked all at once because it is one decision — you are choosing paper, not
/// configuring five settings — and every part has an answer good enough that
/// most people will just press Add.
///
/// The size defaults to the page it will follow, when there is one. A new sheet
/// that does not match its neighbours reads as a mistake in an otherwise uniform
/// document, and that is the common case; the standards are there for when it is
/// not.
struct BlankPageSheet: View {
    /// The page the new sheet will follow, for the "Same as this" option.
    var template: CGSize?
    /// True when this makes a document rather than adding to one. Only then are
    /// "how many" and a file name questions at all — inserting into an open
    /// document has an answer for both already.
    var newDocument: Bool = false
    /// The name offered when a document is being made.
    var suggestedName: String = "Notes"
    let onAdd: (BlankSheet) -> Void
    let onDismiss: () -> Void

    @Environment(\.colorScheme) private var scheme

    @State private var chosen: SheetSize?
    @State private var landscape = false
    @State private var fill: MarkColor = paperWhite
    @State private var ruling: Ruling = .none
    @State private var count = "1"
    @State private var name = ""

    private var sizes: [SheetSize] { sheetSizes(template: template) }
    private var selection: SheetSize { chosen ?? sizes[0] }

    /// nil while the typed count is not a number of pages anyone meant.
    private var pages: Int? {
        guard let typed = Int(count), (1...maximumSheets).contains(typed) else { return nil }
        return typed
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(newDocument ? "New document" : "Add a page")
                .font(.title3.weight(.semibold))
                .padding(.bottom, 16)

            // Scrolls: with a name, a count, sizes, orientation, colour and
            // ruling this is taller than a short phone in landscape, and a dialog
            // that overflows hides its own buttons.
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if newDocument {
                        nameField
                        pagesField
                    }

                    FieldLabel("Size")
                    ChipRow {
                        ForEach(sizes) { sheet in
                            Chip(label: sheet.label, selected: sheet == selection) {
                                chosen = sheet
                            }
                        }
                    }

                    ChipRow {
                        Chip(label: "Portrait", selected: !landscape) { landscape = false }
                        Chip(label: "Landscape", selected: landscape) { landscape = true }
                    }

                    FieldLabel("Colour")
                    ChipRow {
                        ForEach(sheetColours) { paper in
                            Swatch(paper: paper, selected: paper.colour == fill) {
                                fill = paper.colour
                            }
                        }
                    }

                    FieldLabel("Ruling")
                    ChipRow {
                        ForEach(Ruling.allCases) { option in
                            Chip(label: option.label, selected: option == ruling) {
                                ruling = option
                            }
                        }
                    }
                }
            }

            HStack(spacing: 16) {
                Spacer()
                Button("Cancel") { onDismiss() }
                Button(newDocument ? "Create" : "Add") { confirm() }
                    .fontWeight(.semibold)
                    .disabled(pages == nil)
            }
            .padding(.top, 16)
        }
        .padding(20)
        .background(PagifyColor.background(scheme))
        .onAppear { if name.isEmpty { name = suggestedName } }
        // A different page to follow is a different set of offers, so whatever
        // was picked from the old set stops meaning anything.
        .onChange(of: template) { _, _ in chosen = nil }
    }

    private var nameField: some View {
        VStack(alignment: .leading, spacing: 4) {
            FieldLabel("Name")
            TextField("Name", text: $name)
                .textFieldStyle(.roundedBorder)
        }
    }

    private var pagesField: some View {
        VStack(alignment: .leading, spacing: 4) {
            FieldLabel("Pages")
            TextField("Pages", text: $count)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
                .overlay {
                    if pages == nil {
                        RoundedRectangle(cornerRadius: 5).strokeBorder(Color.red, lineWidth: 1)
                    }
                }
                .onChange(of: count) { _, typed in
                    // Digits only, and short: the field is a number, and a paste
                    // of something else should not become one.
                    let digits = String(typed.filter(\.isNumber).prefix(3))
                    if digits != typed { count = digits }
                }
            if pages == nil {
                Text("Between 1 and \(maximumSheets)")
                    .font(.caption)
                    .foregroundStyle(Color.red)
            }
        }
    }

    private func confirm() {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        onAdd(BlankSheet(
            count: pages ?? 1,
            size: landscape ? selection.turnedOnItsSide() : selection,
            // White is what an empty page already looks like, so it is sent as no
            // fill at all rather than as a white rectangle covering the sheet —
            // which would print, and would be there for good.
            fill: fill == paperWhite ? nil : fill,
            ruling: ruling,
            name: trimmed.isEmpty ? suggestedName : trimmed
        ))
    }
}

/// The bridge from the library's `+` to the sheet above.
///
/// `PagifyApp` and `ReaderModel.createBlank` still take four loose arguments
/// where the sheet now hands over one payload. The chosen name goes no further
/// than here, because `createBlank` picks the filename itself.
struct BlankDocumentSheet: View {
    let onCreate: (Int, PaperSize, Ruling, MarkColor?) -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        BlankPageSheet(
            newDocument: true,
            onAdd: { sheet in onCreate(sheet.count, sheet.size, sheet.ruling, sheet.fill) },
            onDismiss: { dismiss() }
        )
    }
}

// ------------------------------------------------------------------- parts --

private struct FieldLabel: View {
    let text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

private struct Chip: View {
    let label: String
    let selected: Bool
    let action: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: action) {
            Text(label)
                // One line, always. A chip whose label wraps is taller than its
                // neighbours and reads as broken rather than as a long word.
                .lineLimit(1)
                .fixedSize()
                .font(.subheadline.weight(.medium))
                .foregroundStyle(selected ? PagifyColor.primary(scheme) : Color.secondary)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(chipBackground, in: RoundedRectangle(cornerRadius: 10))
        }
        .buttonStyle(.plain)
    }

    /// A tinted container rather than a filled one: the selected chip has to read
    /// as chosen next to four unchosen ones, not as the only button on the row.
    private var chipBackground: Color {
        selected
            ? PagifyColor.primary(scheme).opacity(0.18)
            : PagifyColor.surfaceVariant(scheme)
    }
}

/// A colour, shown as the paper itself.
///
/// Outlined rather than filled with a tick: white paper on a white swatch needs
/// an edge to exist at all, and the same edge doing the selecting keeps the row
/// reading as a row of sheets.
private struct Swatch: View {
    let paper: BlankPaper
    let selected: Bool
    let action: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: action) {
            Circle()
                .fill(Color(paper.colour.cgColor))
                .frame(width: 36, height: 36)
                .overlay(
                    Circle().strokeBorder(
                        selected ? PagifyColor.primary(scheme) : Color(.separator),
                        lineWidth: selected ? 3 : 1
                    )
                )
        }
        .buttonStyle(.plain)
        .accessibilityLabel(paper.name)
    }
}

/// A row of chips that wraps.
///
/// Not an `HStack`: five size chips do not fit across a phone, and a stack
/// squeezes the last one until its label breaks across two lines or disappears
/// entirely.
private struct ChipRow<Content: View>: View {
    private let content: () -> Content

    init(@ViewBuilder content: @escaping () -> Content) {
        self.content = content
    }

    var body: some View {
        // Applied through a value rather than by naming the type, so the trailing
        // closure lands on the layout's `callAsFunction` and not on an
        // initialiser that has no room for it.
        let layout = ChipFlow(spacing: 8)
        return layout { content() }
    }
}

private struct ChipFlow: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        // An unspecified width means "how wide would you like to be?", and a
        // flow answers that with the width it was given or a sensible sheet
        // width — never with nil, which would collapse every chip onto one row.
        arrange(subviews, within: proposal.width ?? 320).size
    }

    func placeSubviews(in bounds: CGRect,
                       proposal: ProposedViewSize,
                       subviews: Subviews,
                       cache: inout ()) {
        let placements = arrange(subviews, within: bounds.width).places
        for (view, place) in zip(subviews, placements) {
            let size = view.sizeThatFits(.unspecified)
            view.place(at: CGPoint(x: bounds.minX + place.x, y: bounds.minY + place.y),
                       anchor: .topLeading,
                       proposal: ProposedViewSize(size))
        }
    }

    private func arrange(_ subviews: Subviews,
                         within width: CGFloat) -> (size: CGSize, places: [CGPoint]) {
        var places: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var lineHeight: CGFloat = 0
        var widest: CGFloat = 0

        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            // `x > 0` so a chip wider than the whole row still gets a line of its
            // own rather than being pushed onto an empty one for ever.
            if x > 0, x + size.width > width {
                x = 0
                y += lineHeight + spacing
                lineHeight = 0
            }
            places.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            widest = max(widest, x - spacing)
            lineHeight = max(lineHeight, size.height)
        }

        return (CGSize(width: min(widest, width), height: y + lineHeight), places)
    }
}

// --------------------------------------------------------------- constants --

/// The sizes on offer, with the page being followed first when there is one.
///
/// Deduplicated against that page, so a document that is already A4 does not
/// offer A4 twice under two names.
private func sheetSizes(template: CGSize?) -> [SheetSize] {
    let standards = [
        SheetSize(label: "A4", size: CGSize(width: 595, height: 842)),
        SheetSize(label: "A3", size: CGSize(width: 842, height: 1191)),
        SheetSize(label: "Letter", size: CGSize(width: 612, height: 792)),
        SheetSize(label: "Square", size: CGSize(width: 595, height: 595)),
    ]
    guard let template else { return standards }
    return [SheetSize(label: "Same as this", size: template)]
        + standards.filter { !$0.matches(template) }
}

/// One paper on offer.
struct BlankPaper: Identifiable {
    let name: String
    let colour: MarkColor

    var id: String { name }
}

private let paperWhite = MarkColor(argb: 0xFF_FF_FF_FF)

/// The papers on offer. White first, because it is what paper usually is.
private let sheetColours: [BlankPaper] = [
    BlankPaper(name: "White", colour: paperWhite),
    BlankPaper(name: "Cream", colour: MarkColor(argb: 0xFF_FF_F6_E0)),
    BlankPaper(name: "Grey", colour: MarkColor(argb: 0xFF_BF_C3_C7)),
    BlankPaper(name: "Black", colour: MarkColor(argb: 0xFF_10_12_14)),
    BlankPaper(name: "Blue", colour: MarkColor(argb: 0xFF_1B_3A_5C)),
]

/// As many sheets as one dialog should make in one press.
///
/// Not a technical limit — the engine will build more. A cap, because "500" is
/// far more often a typo than a request, and a dotted sheet carries a couple of
/// thousand objects apiece.
private let maximumSheets = 200
