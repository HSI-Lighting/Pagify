import CoreGraphics
import SwiftUI

/// What the export sheet is being opened for. Only the verb differs.
enum CaptureExportAction: String, Identifiable {
    case save, share, copy

    var id: String { rawValue }

    var verb: String {
        switch self {
        case .save: return "Save"
        case .share: return "Share"
        case .copy: return "Copy"
        }
    }

    var systemImage: String {
        switch self {
        case .save: return "square.and.arrow.down"
        case .share: return "square.and.arrow.up"
        case .copy: return "doc.on.doc"
        }
    }
}

/// The capture, full screen, with everything you can do to it.
///
/// Full screen rather than a sheet because this is a workspace, not a menu: it is
/// where the picture is checked, drawn on and decided about, and a sheet gave the
/// picture a third of the display while the controls took the rest.
///
/// The picture can be pinched to zoom and panned with two fingers — the same
/// gestures as the reader, and for the same reason: drawing a small arrow on a
/// dense page needs the picture bigger than the screen, and the export is at its
/// own resolution regardless of what the display is showing. One finger always
/// draws, which is what makes the two-finger split necessary rather than a
/// nicety: a one-finger drag cannot mean both "draw" and "pan".
///
/// The marks made here are **never written to the document.** They live on the
/// picture, are cleared with it, and reach a file only by the engine drawing them
/// into an exported image — which is why there is no annotation history in this
/// screen beyond dropping the last mark.
struct CaptureEditor: View {
    /// The reader's own backdrop, kept so `CaptureFill.page` can be chosen again
    /// after another fill has overwritten the request's colour. Only the reader
    /// knows it — it comes from the theme — and by the time this is on screen the
    /// reader is not there to ask.
    let readerBackground: MarkColor
    /// Re-render the picture from the document. Nil when the engine could not,
    /// in which case what is on screen is left alone.
    let render: (CaptureRequest) async -> CapturePreview?
    /// Hand the picture, with its marks drawn in by the engine, to Files, a share
    /// sheet or the pasteboard.
    let export: (CaptureExportAction, CaptureRequest, [Markup]) async -> Void
    let onDismiss: () -> Void

    @Environment(\.colorScheme) private var scheme

    @State private var capture: CapturePreview
    @State private var marks: [Markup] = []
    @State private var settings = MarkupSettings()
    @State private var fill: CaptureFill = .page
    @State private var isCapturing = false

    @State private var zoom: CGFloat = 1
    @State private var pan: CGSize = .zero

    /// The baseline waiting for its words.
    ///
    /// Kept here rather than in the reader's state: it lives and dies with this
    /// screen, and a half-typed caption has no meaning once the picture is gone.
    @State private var pendingText: [CGPoint]?
    /// The caption being rewritten, when a second tap opened one.
    @State private var editingIndex: Int?
    /// Which export is being set up, if one is.
    @State private var exporting: CaptureExportAction?
    /// Whether the wheel is open, for a colour the palette does not carry.
    @State private var pickingColour = false

    init(capture: CapturePreview, readerBackground: MarkColor,
         render: @escaping (CaptureRequest) async -> CapturePreview?,
         export: @escaping (CaptureExportAction, CaptureRequest, [Markup]) async -> Void,
         onDismiss: @escaping () -> Void) {
        self.readerBackground = readerBackground
        self.render = render
        self.export = export
        self.onDismiss = onDismiss
        _capture = State(initialValue: capture)
    }

    // The three sheets hang off three different views on purpose. Stacked on one
    // they are one presentation between them, and the second to be asked for
    // silently replaces the first — which is how choosing a colour while a
    // caption is being typed loses the words.
    var body: some View {
        VStack(spacing: 0) {
            header
            picture
            controls
        }
        .background(PagifyColor.surface(scheme))
    }

    // -------------------------------------------------------------- header --

    private var header: some View {
        HStack(spacing: 4) {
            Button(action: onDismiss) {
                Image(systemName: "xmark").font(.system(size: 17, weight: .medium))
            }
            .accessibilityLabel("Discard the picture")

            VStack(alignment: .leading, spacing: 1) {
                Text("Picture").font(.headline)
                Text("Page \(capture.request.originPage + 1) · "
                     + "\(capture.request.scale.label) · "
                     + capture.request.format.fileExtension.uppercased() + " · "
                     + capture.sizeLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 8)

            // Only offered once it would do something. A reset button that is
            // always there invites the question of what it resets.
            if zoom != minimumZoom {
                Button {
                    zoom = minimumZoom
                    pan = .zero
                } label: {
                    Text("1:1").font(.system(size: 15, weight: .semibold))
                }
                .accessibilityLabel("Back to fitting the screen")
            }

            Button(action: undoMarkup) {
                Image(systemName: "arrow.uturn.backward").font(.system(size: 17, weight: .medium))
            }
            .disabled(marks.isEmpty)
            .accessibilityLabel("Undo the last mark")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .sheet(isPresented: $pickingColour) {
            ColourWheelDialog(initial: settings.color,
                              onPick: { colour in
                                  // Choosing a colour restyles nothing: a caption
                                  // on a picture has none of its own, so this
                                  // only arms the next mark.
                                  settings.color = colour
                                  pickingColour = false
                              },
                              onDismiss: { pickingColour = false })
        }
    }

    // ------------------------------------------------------------- picture --

    private var picture: some View {
        ZStack {
            PagifyColor.surfaceVariant(scheme)
            // A checkerboard, and only for a cut-out. Over a plain grey panel
            // "transparent" and "the reader's grey backdrop" look exactly alike,
            // and the whole reason to pick transparent is that it is *not* a
            // colour.
            if fill == .transparent { Checkerboard() }

            CaptureCanvas(
                image: capture.picture,
                crop: capture.request.localBounds,
                markup: marks,
                tool: settings.tool,
                armed: settings.armed,
                color: settings.color,
                size: settings.size,
                style: settings.style,
                onCommit: addMarkup,
                onRecognise: recogniseAndAdd,
                onPlaceText: { pendingText = $0 },
                onMoveText: moveMarkup,
                onSelectText: selectMarkup,
                onEditText: { editingIndex = $0 },
                selectedText: settings.selectedIndex,
                zoom: zoom,
                pan: pan,
                onZoom: pinched,
                onPan: pannedTwoFingers,
                onDoubleTap: doubleTapped)
                .padding(8)

            if isCapturing {
                // Over the picture rather than instead of it: re-rendering at a
                // higher scale takes a moment, and swapping the picture for a
                // spinner reads as the capture having been lost.
                Color.black.opacity(0.3)
                ProgressView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // A second guard, on the area rather than the picture: whatever the
        // canvas does, nothing inside here reaches the controls below it.
        .clipped()
        // One sheet for both: writing a new caption, and rewriting one that a
        // second tap opened. They ask the same question of the same field, and
        // two sheets would be two chances for them to drift apart.
        .sheet(isPresented: writingText) {
            TextEditorSheet(
                font: settings.font,
                size: settings.textSize,
                color: settings.color,
                initial: editingIndex.flatMap { caption(at: $0)?.text } ?? "",
                onCommit: commitText)
        }
    }

    // ------------------------------------------------------------ controls --

    private var controls: some View {
        VStack(spacing: 8) {
            markRibbon

            // How sharp and what kind of file used to sit here, on screen the
            // whole time the picture was being marked up. They are not markup:
            // they are questions about the *file*, and the moment to ask them is
            // the moment there is going to be one.
            HStack(spacing: 8) {
                ForEach([CaptureExportAction.save, .share, .copy]) { action in
                    Button {
                        exporting = action
                    } label: {
                        Label(action.verb, systemImage: action.systemImage)
                            .frame(maxWidth: .infinity)
                    }
                    // Save is the emphasised one: it is what most captures are
                    // for, and the other two are the same picture going
                    // somewhere else.
                    .buttonStyle(.bordered)
                    .tint(action == .save ? PagifyColor.primary(scheme)
                          : PagifyColor.onSurface(scheme))
                    .disabled(isCapturing)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .sheet(item: $exporting) { action in
            CaptureExportSheet(
                action: action,
                scale: capture.request.scale,
                format: capture.request.format,
                fill: fill,
                // A box capture is all page, so there is nothing around it to
                // fill and the question is not asked. A drawn-around one always
                // has an outside.
                hasBareArea: !capture.request.mask.isEmpty,
                isCapturing: isCapturing,
                onScale: setScale,
                onFormat: setFormat,
                onFill: setFill,
                onConfirm: { confirmExport(action) })
        }
    }

    /// The screenshot editor's tools, in the row the reader draws with.
    ///
    /// The same row, not one that looks like it: these are the same questions —
    /// what colour, how heavy, what kind of line, what shape — and two rows built
    /// separately is exactly how the two would have drifted apart. The
    /// highlighter is the one tool the reader's row does not have, and it takes a
    /// slot of its own: it is neither a line nor a way of going round something.
    private var markRibbon: some View {
        let tool = settings.tool
        let writes = tool.writesText
        return MarkRibbon(
            groups: MarkupTool.groups.map { group in
                group.map {
                    RibbonTool(key: $0, icon: $0.ribbonIcon, name: $0.label,
                               // As in the reader: the box and the ellipse are the
                               // cloud with a different ring, and showing all five
                               // in one slot costs the others the room to be
                               // legible.
                               inPreview: $0.inPreview)
                }
            },
            armed: settings.armed ? AnyHashable(tool) : nil,
            colour: settings.color,
            palette: MarkupColors.palette,
            // While words are held the weight slot is a point size and the line
            // type slot is a font, exactly as in the reader — neither a nib width
            // nor a dash means anything to a letter.
            width: writes ? settings.textSize : settings.size,
            widthPresets: writes ? AnnotationMetrics.textPresets : tool.sizePresets,
            widthRange: writes ? AnnotationMetrics.textRange : tool.sizeRange,
            // A wash has no length to break up, so the slot drops out rather than
            // offering five patterns that would every one of them draw the same
            // block.
            lineStyle: tool.hasLineStyle && !writes ? settings.style : nil,
            font: writes ? settings.font : nil,
            onFont: setTextFont,
            curve: tool.bendsText ? settings.curveDegrees : nil,
            onCurve: setTextCurve,
            onTool: { key in
                guard let picked = key as? MarkupTool else { return }
                // Tapping the tool already in hand puts it down. Without that
                // there is no way back to no tool at all, and a finger that lands
                // a moment before its partner in a pinch draws on the picture.
                if settings.tool == picked, settings.armed {
                    settings.disarm()
                } else {
                    settings.select(picked)
                }
            },
            onColour: { settings.color = $0 },
            onWidth: { value in
                if writes {
                    setTextSize(value)
                } else {
                    settings.setSize(value, for: tool)
                }
            },
            onLineStyle: { settings.style = $0 },
            onPickCustomColour: { pickingColour = true },
            // Same control, different question: the highlighter's is how strong
            // the wash is, not how thick the nib.
            widthIsIntensity: tool.isIntensity && !writes,
            // A tool can be put down here, as in the reader. It was not, on the
            // reasoning that a finger on the picture had nothing else it could
            // mean — but while pinching to zoom, a finger that lands a moment
            // before or after its partner is one finger, and every one of those
            // drew on the picture.
            onDisarm: { settings.disarm() })
    }

    /// The face, the size and the bend are **sticky and selective at once**: they
    /// arm the next caption and restyle the one in hand, which is why each goes
    /// through one place rather than being set at the ribbon.
    private func setTextFont(_ font: PagifyFont) {
        settings.font = font
        restyleSelected { $0.rebuilt(font: font) }
    }

    private func setTextSize(_ points: CGFloat) {
        settings.setTextSize(points)
        let reached = settings.textSize
        restyleSelected { $0.rebuilt(sizePoints: reached) }
    }

    private func setTextCurve(_ degrees: CGFloat) {
        settings.curveDegrees = degrees
        restyleSelected { $0.rebuilt(curveDegrees: degrees) }
    }

    // ------------------------------------------------------------- gestures --

    private func pinched(_ factor: CGFloat) {
        // A caption in hand takes the pinch, as in the reader: while one is held
        // two fingers mean "this big", and the picture holds still. Tapping empty
        // picture puts it down and gives the zoom back.
        if settings.selectedIndex != nil {
            scaleSelected(by: factor)
            return
        }
        zoom = min(max(zoom * factor, minimumZoom), maximumZoom)
        if zoom == minimumZoom { pan = .zero }
    }

    /// The pan half of the same gesture. Gating only the zoom left the picture
    /// sliding about under a caption being resized — the two read the same two
    /// fingers, so both have to stand down, not one.
    private func pannedTwoFingers(_ delta: CGSize) {
        guard settings.selectedIndex == nil else { return }
        pan = CGSize(width: pan.width + delta.width, height: pan.height + delta.height)
    }

    /// Double-tap zooms about the tapped point, the same as the reader. A pinch
    /// needs two fingers and a deliberate spread; this is the one-handed way in,
    /// and it matters most here because the picture is what the whole screen is
    /// for.
    private func doubleTapped(_ fromCentre: CGPoint) {
        // Nothing while a caption is in hand: a second tap on one opens it for
        // rewriting instead.
        guard settings.selectedIndex == nil else { return }
        let target = zoom > minimumZoom + zoomEpsilon ? minimumZoom : doubleTapZoom
        let ratio = target / zoom
        // Keep whatever was under the finger under the finger. Without this the
        // picture jumps to its centre and the detail being aimed at is gone.
        pan = target == minimumZoom
            ? .zero
            : CGSize(width: fromCentre.x * (1 - ratio) + pan.width * ratio,
                     height: fromCentre.y * (1 - ratio) + pan.height * ratio)
        zoom = target
    }

    // --------------------------------------------------------------- marks --

    private func caption(at index: Int) -> MarkupTextShape? {
        guard marks.indices.contains(index), case .text(let shape) = marks[index].shape else {
            return nil
        }
        return shape
    }

    private func addMarkup(_ shape: MarkupShape) {
        marks.append(markupFor(shape: shape, tool: settings.tool, color: settings.color,
                               size: settings.size, style: settings.style))
        // A caption you have just written is the one in hand, as in the reader:
        // the ribbon's controls act on it straight away, and two fingers resize
        // it rather than moving the picture underneath. Only words — a stroke has
        // nothing for the controls to change.
        if case .text = shape {
            settings.selectedIndex = marks.count - 1
        } else {
            settings.selectedIndex = nil
        }
    }

    /// Ask the engine what a stroke was, then add whatever it says.
    ///
    /// Only reached when the finger held still before lifting, so a snap is
    /// always something that was asked for. The call is pure geometry — no
    /// document, no lock — but it still happens after the lift rather than during
    /// the drag, so nothing can cost a frame mid-stroke.
    private func recogniseAndAdd(_ points: [CGPoint]) {
        Task {
            let shape = await Task.detached(priority: .userInitiated) {
                recogniseMarkupStroke(points)
            }.value
            addMarkup(shape)
        }
    }

    /// Remove the most recent mark. A snapped shape is one mark, so undoing it
    /// removes the whole snap — which is what somebody who did not want the shape
    /// is reaching for.
    private func undoMarkup() {
        guard !marks.isEmpty else { return }
        marks.removeLast()
        if let selected = settings.selectedIndex, selected >= marks.count {
            settings.selectedIndex = nil
        }
    }

    /// Move the words at `index` by `delta` capture units.
    ///
    /// By position rather than by identity, because a capture mark has none: the
    /// list *is* the drawing, in the order it was drawn. Nothing reorders it, so
    /// the index a drag started on is the mark it started on.
    private func moveMarkup(_ index: Int, _ delta: CGPoint) {
        guard delta != .zero, let shape = caption(at: index) else { return }
        marks[index].shape = .text(shape.movedBy(delta))
    }

    /// Pick up the caption at `index`, or put it down with a negative one.
    private func selectMarkup(_ index: Int) {
        guard index >= 0, let shape = caption(at: index) else {
            settings.selectedIndex = nil
            return
        }
        settings.selectedIndex = index
        // The ribbon's controls are now about this caption, so they have to show
        // what it is rather than what the tool was last set to.
        settings.font = shape.font
        settings.textSize = shape.sizePoints
        settings.curveDegrees = shape.curveDegrees
        settings.color = marks[index].color
    }

    /// Resize the caption in hand by `factor`.
    ///
    /// The pinch's own arithmetic: a factor rather than a size, because that is
    /// what two fingers say. Held to what the picture can carry — a run wider
    /// than the picture is words nobody can read.
    private func scaleSelected(by factor: CGFloat) {
        guard factor != 1, let index = settings.selectedIndex,
              let shape = caption(at: index) else { return }

        let across = capture.request.localBounds.width * AnnotationMetrics.textPageFraction
        let ceiling = sizeThatFits(shape.text, font: shape.font, availableWidth: across)
        marks[index].shape = .text(shape.rebuilt(sizePoints: min(shape.sizePoints * factor,
                                                                 ceiling)))
        // The slider follows the pinch, so the two controls never disagree about
        // how big the caption in hand is.
        if let reached = caption(at: index) { settings.textSize = reached.sizePoints }
    }

    /// Restyle the caption in hand, if there is one.
    ///
    /// **No colour argument anywhere in this funnel.** A capture caption has no
    /// colour of its own — the enclosing mark carries it — so choosing a colour
    /// restyles nothing and simply arms the next mark.
    private func restyleSelected(_ change: (MarkupTextShape) -> MarkupTextShape) {
        guard let index = settings.selectedIndex, let shape = caption(at: index) else { return }
        marks[index].shape = .text(change(shape))
    }

    /// The words came back from the sheet.
    private func commitText(_ words: String) {
        defer {
            pendingText = nil
            editingIndex = nil
        }

        if let index = editingIndex {
            // Blank words are how a caption is deleted: nothing to see, and
            // nothing to rub out except by guessing where it was.
            if words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                eraseMarkup(index)
            } else {
                guard let shape = caption(at: index) else { return }
                marks[index].shape = .text(shape.rebuilt(text: words))
            }
            return
        }

        guard let anchor = pendingText?.first,
              !words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }

        // A tap gives one point and the layout walks a line; the bend is a
        // setting, so the line is built rather than traced.
        let bend = settings.tool.bendsText ? settings.curveDegrees : 0
        addMarkup(.text(MarkupTextShape(
            text: words,
            path: curvedBaseline(anchor: anchor, text: words, font: settings.font,
                                 size: settings.textSize, degrees: bend),
            font: settings.font,
            sizePoints: settings.textSize,
            frame: settings.tool.textFrame,
            curveDegrees: bend)))
    }

    /// Remove the caption at `index`. The list is the drawing, so this takes it
    /// out of the list — and anything selected after it shifts down by one.
    private func eraseMarkup(_ index: Int) {
        guard marks.indices.contains(index) else { return }
        marks.remove(at: index)
        settings.selectedIndex = nil
    }

    private var writingText: Binding<Bool> {
        Binding(get: { pendingText != nil || editingIndex != nil },
                set: { open in
                    if !open {
                        pendingText = nil
                        editingIndex = nil
                    }
                })
    }

    // -------------------------------------------------------------- export --

    private func setScale(_ scale: CaptureScale) {
        guard capture.request.scale != scale else { return }
        var request = capture.request
        request.scale = scale
        retake(request)
    }

    private func setFormat(_ format: CaptureFormat) {
        guard capture.request.format != format else { return }
        var request = capture.request
        request.format = format
        retake(request)
    }

    /// Choose what fills the capture where no page reaches.
    ///
    /// Re-renders what is on screen rather than only applying to the next
    /// capture: the fill is a decision about the picture in front of you, and the
    /// way to judge it is to see it.
    private func setFill(_ next: CaptureFill) {
        guard fill != next else { return }
        fill = next
        var request = capture.request
        request.background = next.colour.map { MarkColor(argb: $0) } ?? readerBackground
        // Picking a cut-out also moves the export to PNG, because JPEG has no
        // alpha channel: leaving it on JPEG would flatten the cut-out back to a
        // colour and hand back a picture that quietly ignored the choice.
        if next == .transparent { request.format = .png }
        retake(request)
    }

    private func retake(_ request: CaptureRequest) {
        guard !isCapturing else { return }
        isCapturing = true
        Task {
            let taken = await render(request)
            if let taken { capture = taken }
            isCapturing = false
        }
    }

    private func confirmExport(_ action: CaptureExportAction) {
        let request = capture.request
        let committed = marks
        Task { await export(action, request, committed) }
    }
}

/// The backdrop that says "nothing here".
///
/// The same two-tone grid every image editor uses, for the same reason: it is the
/// one pattern nobody mistakes for part of the picture.
private struct Checkerboard: View {
    var body: some View {
        Canvas { context, size in
            let step: CGFloat = 24
            context.fill(Path(CGRect(origin: .zero, size: size)),
                         with: .color(Color(hex: 0x3A3A3E)))
            var row = 0
            var y: CGFloat = 0
            while y < size.height {
                var column = 0
                var x: CGFloat = 0
                while x < size.width {
                    if (row + column) % 2 == 0 {
                        context.fill(
                            Path(CGRect(x: x, y: y,
                                        width: min(step, size.width - x),
                                        height: min(step, size.height - y))),
                            with: .color(Color(hex: 0x2C2C30)))
                    }
                    x += step
                    column += 1
                }
                y += step
                row += 1
            }
        }
    }
}

/// Below 1× there is nothing more to see; the picture already fits.
private let minimumZoom: CGFloat = 1
/// Past this the preview is being magnified rather than resolved: it is decoded
/// downsampled, so more zoom only enlarges pixels.
private let maximumZoom: CGFloat = 8
/// Where a double tap lands. The reader's figure, so the two feel like one app.
private let doubleTapZoom: CGFloat = 2.5
/// Float slack for "is it zoomed": a pinch rarely leaves the scale at exactly 1.
private let zoomEpsilon: CGFloat = 0.01

// ---------------------------------------------------------- the export sheet --

/// How sharp, what kind of file, and what fills the bare parts — asked once, at
/// the moment it matters.
///
/// These used to sit on screen the whole time a picture was being marked up,
/// which put three questions about the *file* in front of somebody drawing an
/// arrow. The answers are remembered, so somebody who always sends the same kind
/// of file is confirming a sheet that is already right rather than answering
/// again.
private struct CaptureExportSheet: View {
    let action: CaptureExportAction
    let scale: CaptureScale
    let format: CaptureFormat
    let fill: CaptureFill
    /// Whether the picture has any area no page reaches.
    let hasBareArea: Bool
    let isCapturing: Bool
    let onScale: (CaptureScale) -> Void
    let onFormat: (CaptureFormat) -> Void
    let onFill: (CaptureFill) -> Void
    let onConfirm: () -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section("Quality") {
                    Picker("Quality", selection: Binding(get: { scale }, set: onScale)) {
                        ForEach(CaptureScale.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }

                Section("Format") {
                    Picker("Format", selection: Binding(get: { format }, set: onFormat)) {
                        ForEach(CaptureFormat.allCases) {
                            Text($0.fileExtension.uppercased()).tag($0)
                        }
                    }
                    .pickerStyle(.segmented)
                }

                // Only for a picture that has an outside. A box capture inside
                // one page is all page, so the question would have nothing to act
                // on.
                if hasBareArea {
                    Section("Around it") {
                        Picker("Around it", selection: Binding(get: { fill }, set: onFill)) {
                            ForEach(CaptureFill.allCases) { Text($0.label).tag($0) }
                        }
                        .pickerStyle(.segmented)
                    }
                }
            }
            .disabled(isCapturing)
            .navigationTitle("\(action.verb) this picture")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action.verb) {
                        onConfirm()
                        dismiss()
                    }
                    .fontWeight(.semibold)
                    // Held back while the picture is being re-rendered: changing
                    // the sharpness takes a fresh capture, and exporting before it
                    // lands would hand over a file that is not the one asked for.
                    .disabled(isCapturing)
                }
            }
        }
        .presentationDetents([.medium])
    }
}
