import CoreGraphics
import CoreText
import SwiftUI
import UIKit

/// The capture, with its marks drawn over it and the input that adds more.
///
/// The picture on screen is a downscaled preview; the marks are drawn on top of
/// it rather than composited into it, so a stroke follows the finger at frame rate
/// and no engine call happens while one is down. The exported file is rendered
/// separately, by the engine, from these same shapes.
///
/// Everything crossing the boundary is in **capture units**. The mapping is the
/// displayed rectangle onto the picture's own bounds, so it holds however the
/// preview happens to be scaled or letterboxed.
struct CaptureCanvas: View {
    let image: CGImage
    /// The picture's own bounds, in capture units. Marks are placed against it.
    let crop: PageRect
    let markup: [Markup]
    let tool: MarkupTool
    /// Whether the tool is held. Nothing draws when it is not.
    let armed: Bool
    let color: MarkColor
    /// Nib width, or the highlighter's intensity.
    let size: CGFloat
    let style: MarkupStyle

    let onCommit: (MarkupShape) -> Void
    /// Held still before lifting: ask the engine what this stroke was.
    let onRecognise: ([CGPoint]) -> Void
    /// A baseline for words has been placed; ask for them.
    let onPlaceText: ([CGPoint]) -> Void
    /// Words already on the picture have been dragged somewhere else.
    let onMoveText: (Int, CGPoint) -> Void
    /// A caption was tapped, so the ribbon's controls now belong to it. A
    /// negative index puts the one in hand down.
    let onSelectText: (Int) -> Void
    /// A caption was tapped twice; rewrite its words.
    let onEditText: (Int) -> Void
    /// Which caption the ribbon is editing, drawn picked out.
    let selectedText: Int?

    /// Magnification of the picture on screen. Nothing to do with the export
    /// scale: this is for looking closely and drawing accurately, and the file is
    /// rendered at its own resolution regardless.
    let zoom: CGFloat
    let pan: CGSize
    /// Two fingers on the picture. Reported in the area's own points, so the
    /// editor's zoom arithmetic needs no notion of the canvas's transform.
    let onZoom: (CGFloat) -> Void
    let onPan: (CGSize) -> Void
    /// A double tap, as an offset from the middle of the picture area.
    let onDoubleTap: (CGPoint) -> Void

    /// The wet stroke, rebuilt every frame by the same code that will commit it.
    @State private var preview: [MarkupShape] = []
    @State private var dwelling = false
    /// Which mark is being dragged, and how far it has come.
    @State private var movingIndex = -1
    @State private var moveShift = CGPoint.zero
    /// When the last tap on a caption lifted, and which one, for double taps.
    @State private var lastTapAt: TimeInterval = 0
    @State private var lastTapIndex = -1
    /// The caption a text-tool drag picked up, decided on the way down.
    @State private var grabbed = -1
    @State private var strayed = false
    @State private var strokeOrigin = CGPoint.zero
    @State private var gesture: MarkupGesture?

    var body: some View {
        GeometryReader { geometry in
            // Fit the picture inside the available space, then work in that
            // rectangle. Deriving the mapping from the *displayed* size rather
            // than the preview's pixel size keeps it right whatever the decoder
            // chose to downsample to.
            let aspect = CGFloat(image.width) / CGFloat(max(image.height, 1))
            let boxAspect = geometry.size.width / max(geometry.size.height, 1)
            let shownWidth = aspect >= boxAspect ? geometry.size.width
                                                 : geometry.size.height * aspect
            let shownHeight = aspect >= boxAspect ? geometry.size.width / max(aspect, 0.0001)
                                                  : geometry.size.height

            ZStack {
                Image(decorative: image, scale: 1)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: shownWidth, height: shownHeight)
                    .accessibilityLabel("The captured region")

                Canvas { context, _ in
                    draw(in: context, shownWidth: shownWidth, shownHeight: shownHeight)
                }
                .frame(width: shownWidth, height: shownHeight)

                CaptureInputLayer(
                    // No tool held, no stroke input at all. A disabled handler
                    // that consumed the touches and did nothing would still
                    // swallow the pinch this exists to protect.
                    strokes: armed,
                    onDown: { down($0, width: shownWidth, height: shownHeight) },
                    onMove: { moved($0, width: shownWidth, height: shownHeight) },
                    onUp: { up($0, width: shownWidth, height: shownHeight) },
                    onCancel: cancelStroke,
                    onDwell: dwell,
                    onZoom: onZoom,
                    // The layer sits inside the transform, so its translations are
                    // in the zoomed picture's points; the editor works in the
                    // area's, which is what the zoom is relative to.
                    onPan: { onPan(CGSize(width: $0.width * zoom, height: $0.height * zoom)) },
                    onDoubleTap: { point in
                        let centred = CGPoint(x: point.x - shownWidth / 2,
                                              y: point.y - shownHeight / 2)
                        onDoubleTap(CGPoint(x: centred.x * zoom + pan.width,
                                            y: centred.y * zoom + pan.height))
                    })
                    .frame(width: shownWidth, height: shownHeight)

                if dwelling {
                    // Says the hold registered and what lifting will now do.
                    // Without it a snap arrives unannounced, which is the one
                    // thing recognition must never do.
                    DwellHint()
                        .frame(width: shownWidth, height: shownHeight, alignment: .top)
                        .allowsHitTesting(false)
                }
            }
            .frame(width: shownWidth, height: shownHeight)
            .scaleEffect(zoom)
            .offset(pan)
            .frame(width: geometry.size.width, height: geometry.size.height)
            // Clipped to the area, because a mark past the picture's edge is not
            // in the export either — showing it would be a promise the file does
            // not keep.
            .clipped()
        }
        // A gesture belongs to one tool, and carrying a half-drawn stroke into
        // another would commit it as the wrong shape. The size matters too: the
        // cloud sizes its scallops from it, and a gesture holding the number from
        // before would draw a cloud nobody asked for.
        .onChange(of: tool) { _, _ in resetGesture() }
        .onChange(of: size) { _, _ in resetGesture() }
    }

    // ------------------------------------------------------------- drawing --

    private func mapping(width: CGFloat, height: CGFloat) -> CaptureMapping {
        CaptureMapping(crop: crop, width: width, height: height)
    }

    private func draw(in context: GraphicsContext, shownWidth: CGFloat, shownHeight: CGFloat) {
        let map = mapping(width: shownWidth, height: shownHeight)

        for (index, mark) in markup.enumerated() {
            var shown = mark.shape
            if case .text(let caption) = shown, index == movingIndex {
                shown = .text(caption.movedBy(moveShift))
            }
            draw(shape: shown, color: mark.color,
                 widthPoints: mark.widthPoints * map.scale, style: mark.style,
                 map: map, in: context)

            if case .text(let caption) = shown, index == selectedText {
                draw(selection: caption, map: map, in: context)
            }
        }

        // The wet stroke goes through the same builder the commit uses, so what
        // is under the finger is what ends up in the file — including the
        // highlighter's intensity, which rides in the colour's alpha.
        for shape in preview {
            let wet = markupFor(shape: shape, tool: tool, color: color, size: size, style: style)
            draw(shape: wet.shape, color: wet.color,
                 widthPoints: wet.widthPoints * map.scale, style: wet.style,
                 map: map, in: context)
        }
    }

    private func draw(shape: MarkupShape, color: MarkColor, widthPoints: CGFloat,
                      style: MarkupStyle, map: CaptureMapping,
                      in context: GraphicsContext) {
        let ink = Color(color.cgColor)
        let width = max(widthPoints, 1)
        let stroke = StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round,
                                 dash: style.dashPattern(width: width))

        switch shape {
        case .text(let caption):
            draw(caption: caption, ink: ink, map: map, in: context)

        case .freehand(let points):
            guard points.count >= 2 else { return }
            var path = Path()
            path.move(to: map.toDisplay(points[0]))
            // Quadratics through the midpoints, so a traced stroke reads as one
            // curve rather than as the sampling rate of the glass it was drawn on.
            for index in 1..<points.count {
                let previous = map.toDisplay(points[index - 1])
                let current = map.toDisplay(points[index])
                path.addQuadCurve(to: CGPoint(x: (previous.x + current.x) / 2,
                                              y: (previous.y + current.y) / 2),
                                  control: previous)
            }
            path.addLine(to: map.toDisplay(points[points.count - 1]))
            context.stroke(path, with: .color(ink), style: stroke)

        case .line(let from, let to):
            var path = Path()
            path.move(to: map.toDisplay(from))
            path.addLine(to: map.toDisplay(to))
            context.stroke(path, with: .color(ink), style: stroke)

        case .arrow(let from, let to):
            let start = map.toDisplay(from)
            let tip = map.toDisplay(to)
            var shaft = Path()
            shaft.move(to: start)
            shaft.addLine(to: tip)
            context.stroke(shaft, with: .color(ink), style: stroke)

            // The barbs are drawn solid and separately, so the tip keeps its
            // point: a single polyline out to one barb and back rounds off
            // exactly where an arrow needs to be sharp.
            let angle = atan2(tip.y - start.y, tip.x - start.x)
            let head = width * arrowHeadWidths
            for side in [CGFloat(-1), 1] {
                let barb = angle + .pi + side * arrowHeadAngle
                var path = Path()
                path.move(to: tip)
                path.addLine(to: CGPoint(x: tip.x + head * cos(barb),
                                         y: tip.y + head * sin(barb)))
                context.stroke(path, with: .color(ink),
                               style: StrokeStyle(lineWidth: width, lineCap: .round))
            }

        case .rectangle(let rect):
            context.stroke(Path(map.toDisplay(rect)), with: .color(ink), style: stroke)

        case .ellipse(let rect):
            context.stroke(Path(ellipseIn: map.toDisplay(rect)), with: .color(ink), style: stroke)

        case .highlight(let rect):
            // Filled and translucent, matching what the engine composites: a
            // highlight that covers what it marks has failed at its one job. The
            // alpha is the mark's own — the intensity slider sets it, and the
            // engine builds the export from the same number.
            context.fill(Path(map.toDisplay(rect)), with: .color(ink))
        }
    }

    /// Words on the picture, drawn glyph by glyph.
    ///
    /// Through Core Text rather than the outlines the export flattens them to: on
    /// screen this has to be sharp at any zoom, and the outlines exist only
    /// because a file cannot hold text. Both walk the same layout, so what is on
    /// screen is where the letters land.
    private func draw(caption: MarkupTextShape, ink: Color, map: CaptureMapping,
                      in context: GraphicsContext) {
        guard !caption.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !caption.path.isEmpty else { return }

        // The frame first, so the words sit over it where they meet.
        let ring = caption.frameOutline()
        if ring.count >= 2 {
            var path = Path()
            path.move(to: map.toDisplay(ring[0]))
            for point in ring.dropFirst() { path.addLine(to: map.toDisplay(point)) }
            context.stroke(
                path, with: .color(ink),
                style: StrokeStyle(
                    lineWidth: max(caption.sizePoints * textFrameStroke * map.scale, 1),
                    lineCap: .round, lineJoin: .round))
        }

        let drawn = caption.sizePoints * map.scale
        let uiFont = caption.font.uiFont(size: drawn)
        // Built by name rather than bridged from the `UIFont`: the bridge is only
        // guaranteed one way, and the glyph ids have to come from the same file
        // the shaper measured.
        let ctFont = CTFontCreateWithName(uiFont.fontName as CFString, uiFont.pointSize, nil)
        let tint = UIColor(ink)

        context.withCGContext { cg in
            cg.setFillColor(tint.cgColor)
            for placement in caption.layOutBlock() {
                let at = map.toDisplay(placement.origin)
                cg.saveGState()
                cg.translateBy(x: at.x, y: at.y)
                cg.rotate(by: placement.radians)
                // Core Text lays type out with y increasing upwards; the capture
                // space runs downwards, so the flip happens here rather than at
                // every coordinate.
                cg.scaleBy(x: 1, y: -1)

                if caption.font.isEmbedded, placement.id != 0 {
                    // By glyph id, not by character: a joined Arabic form has no
                    // character of its own to hand a string-drawing API, and
                    // passing the letters instead is what made Persian preview as
                    // a row of isolated shapes.
                    var glyph = CGGlyph(truncatingIfNeeded: placement.id)
                    var origin = CGPoint.zero
                    CTFontDrawGlyphs(ctFont, &glyph, &origin, 1, cg)
                } else {
                    let run = NSAttributedString(
                        string: placement.text,
                        attributes: [.font: uiFont, .foregroundColor: tint])
                    cg.textPosition = .zero
                    CTLineDraw(CTLineCreateWithAttributedString(run), cg)
                }
                cg.restoreGState()
            }
        }
    }

    /// The caption the ribbon is editing, picked out.
    ///
    /// The same dashed amber box the reader draws round a selected caption, for
    /// the same reason: without it the controls are visibly about nothing.
    private func draw(selection caption: MarkupTextShape, map: CaptureMapping,
                      in context: GraphicsContext) {
        guard !caption.path.isEmpty else { return }
        // The gap is in screen points at any zoom, so it is taken back out of
        // capture units before the box is inflated.
        let gap = captionSelectionGap / max(map.scale, 0.01)
        let box = caption.runBounds().inflatedBy(gap)
        let corner = map.toDisplay(CGPoint(x: box.left, y: box.top))
        let far = map.toDisplay(CGPoint(x: box.right, y: box.bottom))
        let width = min(max(caption.sizePoints * captionSelectionStroke * map.scale, 1.5), 6)

        context.stroke(
            Path(CGRect(x: corner.x, y: corner.y,
                        width: far.x - corner.x, height: far.y - corner.y)),
            with: .color(captionSelectionInk),
            style: StrokeStyle(lineWidth: width, dash: [width * 3, width * 2.5]))
    }

    // --------------------------------------------------------------- input --

    private func resetGesture() {
        gesture = nil
        preview = []
        dwelling = false
        movingIndex = -1
        moveShift = .zero
        grabbed = -1
        lastTapAt = 0
        lastTapIndex = -1
    }

    private func down(_ point: CGPoint, width: CGFloat, height: CGFloat) {
        let map = mapping(width: width, height: height)
        let at = map.toCapture(point)
        strokeOrigin = point
        strayed = false

        guard !tool.writesText else {
            // Text is not dragged out, so it does not go through the gesture
            // machine at all: a tap places a baseline, a drag on words already
            // there moves them.
            grabbed = markup.lastIndex { mark in
                guard case .text(let caption) = mark.shape else { return false }
                return caption.isHitBy(at, tolerance: MarkupMetrics.textGrabPoints)
            } ?? -1
            moveShift = .zero
            movingIndex = -1
            return
        }

        let live = MarkupGesture(tool: tool, sizePoints: size)
        live.down(at: at)
        gesture = live
        preview = []
        dwelling = false
    }

    private func moved(_ point: CGPoint, width: CGFloat, height: CGFloat) {
        let map = mapping(width: width, height: height)
        if hypot(point.x - strokeOrigin.x, point.y - strokeOrigin.y) > touchSlop {
            strayed = true
        }

        guard !tool.writesText else {
            guard grabbed >= 0 else { return }
            let from = map.toCapture(strokeOrigin)
            let to = map.toCapture(point)
            movingIndex = grabbed
            moveShift = CGPoint(x: to.x - from.x, y: to.y - from.y)
            return
        }

        guard let gesture else { return }
        gesture.move(to: map.toCapture(point))
        dwelling = gesture.isDwelling
        preview = gesture.preview
    }

    private func up(_ point: CGPoint, width: CGFloat, height: CGFloat) {
        let map = mapping(width: width, height: height)

        guard !tool.writesText else {
            defer {
                grabbed = -1
                movingIndex = -1
                moveShift = .zero
            }
            guard grabbed >= 0 else {
                // A drag that started on nothing is a drag, not a tap, and
                // placing words at the end of one would be a mark nobody asked
                // for.
                guard !strayed else { return }
                if selectedText != nil {
                    // A caption in hand: put it down. That is also how the
                    // picture gets its pinch back, since a held caption takes it.
                    onSelectText(-1)
                } else {
                    onPlaceText([map.toCapture(point)])
                }
                return
            }

            if movingIndex >= 0 {
                onMoveText(grabbed, moveShift)
                lastTapAt = 0
                return
            }

            // A press that went nowhere is a tap, and a tap on words picks them
            // up for the ribbon to work on. A second one soon after opens them
            // for rewriting — counted here rather than by a tap recogniser,
            // because this layer claims the press first and a recogniser waiting
            // for an unclaimed one would never fire.
            let now = Date.timeIntervalSinceReferenceDate
            if now - lastTapAt < doubleTapSeconds, lastTapIndex == grabbed {
                onEditText(grabbed)
                lastTapAt = 0
            } else {
                onSelectText(grabbed)
                lastTapAt = now
                lastTapIndex = grabbed
            }
            return
        }

        guard let gesture else { return }
        gesture.move(to: map.toCapture(point))
        switch gesture.up() {
        case .commit(let shapes):
            for shape in shapes { onCommit(shape) }
        case .recognise(let points):
            onRecognise(points)
        case .nothing:
            break
        }
        self.gesture = nil
        preview = []
        dwelling = false
    }

    private func cancelStroke() {
        gesture?.cancel()
        gesture = nil
        preview = []
        dwelling = false
        grabbed = -1
        movingIndex = -1
        moveShift = .zero
    }

    private func dwell() {
        guard let gesture else { return }
        gesture.still()
        dwelling = gesture.isDwelling
    }
}

/// Where the hold is announced.
private struct DwellHint: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Text("Release to snap to a shape")
            .font(.system(size: 12, weight: .medium))
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(PagifyColor.onSurface(scheme),
                        in: RoundedRectangle(cornerRadius: 12))
            .foregroundStyle(PagifyColor.surface(scheme))
            .padding(.top, 8)
    }
}

/// Displayed points to capture units and back.
///
/// One value rather than two closures so the two can never be built from
/// different numbers, which is how a mark ends up drawn somewhere other than
/// where it was committed.
struct CaptureMapping {
    let crop: PageRect
    let width: CGFloat
    let height: CGFloat

    /// Capture units per displayed point, for the one mark whose size is not a
    /// stroke width.
    var scale: CGFloat { crop.width > 0 ? width / crop.width : 1 }

    func toCapture(_ point: CGPoint) -> CGPoint {
        CGPoint(x: crop.left + (point.x / max(width, 1)) * crop.width,
                y: crop.top + (point.y / max(height, 1)) * crop.height)
    }

    func toDisplay(_ point: CGPoint) -> CGPoint {
        CGPoint(x: (point.x - crop.left) / max(crop.width, 0.0001) * width,
                y: (point.y - crop.top) / max(crop.height, 0.0001) * height)
    }

    func toDisplay(_ rect: PageRect) -> CGRect {
        let corner = toDisplay(CGPoint(x: rect.left, y: rect.top))
        let far = toDisplay(CGPoint(x: rect.right, y: rect.bottom))
        return CGRect(x: corner.x, y: corner.y, width: far.x - corner.x, height: far.y - corner.y)
    }
}

// ------------------------------------------------------------------- input --

/// One finger draws; two fingers zoom and pan.
///
/// UIKit rather than SwiftUI gestures because that split cannot be expressed in
/// SwiftUI: a `DragGesture` cannot say "only while there is exactly one touch",
/// and there is no two-finger pan at all. Raw touches carry the stroke, and the
/// three recognisers alongside them cancel it the moment a second finger makes
/// the gesture mean something else.
struct CaptureInputLayer: UIViewRepresentable {
    /// Whether a tool is held. False leaves stroke touches alone; the two-finger
    /// gestures keep working, because looking closely at a picture is not
    /// drawing on it.
    let strokes: Bool
    let onDown: (CGPoint) -> Void
    let onMove: (CGPoint) -> Void
    let onUp: (CGPoint) -> Void
    let onCancel: () -> Void
    /// The finger has been still for the dwell.
    let onDwell: () -> Void
    let onZoom: (CGFloat) -> Void
    let onPan: (CGSize) -> Void
    let onDoubleTap: (CGPoint) -> Void

    func makeUIView(context: Context) -> UIView {
        let view = CaptureInputView()
        view.backgroundColor = .clear
        view.isMultipleTouchEnabled = true
        view.apply(self)

        let pinch = UIPinchGestureRecognizer(target: context.coordinator,
                                             action: #selector(Coordinator.pinched(_:)))
        let pan = UIPanGestureRecognizer(target: context.coordinator,
                                         action: #selector(Coordinator.panned(_:)))
        pan.minimumNumberOfTouches = 2
        pan.maximumNumberOfTouches = 2
        let doubleTap = UITapGestureRecognizer(target: context.coordinator,
                                               action: #selector(Coordinator.doubleTapped(_:)))
        doubleTap.numberOfTapsRequired = 2
        // Otherwise every single tap's lift is held back for the double-tap
        // interval, and a caption placed by tapping the picture appears a third
        // of a second after the finger leaves it.
        doubleTap.delaysTouchesEnded = false

        for recogniser in [pinch, pan, doubleTap] as [UIGestureRecognizer] {
            view.addGestureRecognizer(recogniser)
        }
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        (view as? CaptureInputView)?.apply(self)
        context.coordinator.input = self
    }

    func makeCoordinator() -> Coordinator { Coordinator(input: self) }

    final class Coordinator {
        /// Re-read on every update rather than captured once: the closures below
        /// are rebuilt whenever the editor's state changes, and a recogniser
        /// holding the first set would zoom a picture that had been replaced.
        var input: CaptureInputLayer
        private var lastScale: CGFloat = 1

        init(input: CaptureInputLayer) { self.input = input }

        @objc func pinched(_ recogniser: UIPinchGestureRecognizer) {
            switch recogniser.state {
            case .began:
                lastScale = recogniser.scale
                input.onCancel()
            case .changed:
                guard lastScale > 0 else { return }
                input.onZoom(recogniser.scale / lastScale)
                lastScale = recogniser.scale
            default:
                lastScale = 1
            }
        }

        @objc func panned(_ recogniser: UIPanGestureRecognizer) {
            switch recogniser.state {
            case .began:
                recogniser.setTranslation(.zero, in: recogniser.view)
                input.onCancel()
            case .changed:
                let step = recogniser.translation(in: recogniser.view)
                recogniser.setTranslation(.zero, in: recogniser.view)
                input.onPan(CGSize(width: step.x, height: step.y))
            default:
                break
            }
        }

        @objc func doubleTapped(_ recogniser: UITapGestureRecognizer) {
            guard recogniser.state == .ended, let view = recogniser.view else { return }
            input.onDoubleTap(recogniser.location(in: view))
        }
    }
}

/// The stroke half: raw touches, and the dwell that a still finger produces no
/// events to announce.
private final class CaptureInputView: UIView {
    /// Named `input` rather than `layer`, which is `UIView`'s own.
    private var input: CaptureInputLayer?
    private var tracking: UITouch?
    private var dwellTimer: Timer?

    func apply(_ input: CaptureInputLayer) { self.input = input }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesBegan(touches, with: event)
        guard let input, input.strokes, let touch = touches.first else { return }

        // A second finger means this was never a stroke. Abandoning it here as
        // well as on the recognisers' `began` is what stops a mark being left
        // behind by the finger that landed a moment before its partner.
        if tracking != nil || (event?.allTouches?.count ?? 1) > 1 {
            stopTracking()
            input.onCancel()
            return
        }

        tracking = touch
        input.onDown(touch.location(in: self))
        armDwell()
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesMoved(touches, with: event)
        guard let input, let touch = tracking, touches.contains(touch) else { return }
        input.onMove(touch.location(in: self))
        armDwell()
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesEnded(touches, with: event)
        guard let input, let touch = tracking, touches.contains(touch) else { return }
        let at = touch.location(in: self)
        stopTracking()
        input.onUp(at)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesCancelled(touches, with: event)
        guard tracking != nil else { return }
        stopTracking()
        input?.onCancel()
    }

    /// A still finger produces no touch events at all, so a timer *is* the dwell:
    /// nothing polls, and nothing runs between frames.
    private func armDwell() {
        dwellTimer?.invalidate()
        dwellTimer = Timer.scheduledTimer(withTimeInterval: MarkupMetrics.dwellSeconds,
                                          repeats: false) { [weak self] _ in
            self?.input?.onDwell()
        }
    }

    private func stopTracking() {
        dwellTimer?.invalidate()
        dwellTimer = nil
        tracking = nil
    }
}

/// Arrow head length, as a multiple of the stroke width, and its half-angle.
/// Matches the engine's own figures, so the preview is what gets exported.
private let arrowHeadWidths: CGFloat = 4
private let arrowHeadAngle: CGFloat = 0.44

/// How far a finger may wander and still be a tap.
private let touchSlop: CGFloat = 10
/// How long two taps on one caption may be apart and still be a double tap.
private let doubleTapSeconds: TimeInterval = 0.3

/// How far the selection box stands off the words, in screen points at any zoom.
private let captionSelectionGap: CGFloat = 6
private let captionSelectionStroke: CGFloat = 0.09
private let captionSelectionInk = Color(hex: 0xF2A93B)
