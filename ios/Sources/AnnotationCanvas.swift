import CoreText
import SwiftUI

/// The layer that turns a drag into a mark.
///
/// Coordinates are converted to **page points** the moment they arrive, so
/// everything downstream — the shape geometry, the wire format, the engine — sees
/// one space and no one has to remember which. A mark's width is in page points
/// too, so it scales with the page rather than with the zoom it was drawn at.
struct AnnotationCanvas: View {
    let pageIndex: Int
    /// Where this page is drawn and how big.
    ///
    /// A mapping rather than a size, because the magnified view draws the page
    /// wherever the reader has panned it to. Assuming an origin of zero is what
    /// made every tool silently miss once a page was zoomed into: the touches
    /// were converted as though the page were still at the top-left of a row.
    let mapping: PageMapping
    let settings: AnnotationSettings
    let onCommit: (WireAnnotation) -> Void
    /// Where to rub out, and how near counts — the second in page points, since
    /// only this view knows the scale the page is drawn at.
    let onErase: (CGPoint, CGFloat) -> Void
    var onEraseStart: () -> Void = {}
    var onEraseEnd: () -> Void = {}
    /// Where a caption was put down. The words are not known yet — a text tool
    /// places a point and then asks — so this is a separate signal from a commit.
    var onPlaceText: (CGPoint, CGPoint?) -> Void = { _, _ in }
    /// The marks already committed on this page, drawn by the app at the width it
    /// holds — see `ReaderModel.marks` for why the engine cannot be asked.
    var committed: [WireAnnotation] = []
    /// The caption in hand, if any. Read live rather than captured — a snapshot
    /// taken when the gesture was installed goes stale the moment anything is
    /// selected, and the tap rule below then does the wrong thing.
    var selectedText: Int32?
    var onSelectText: (Int32?) -> Void = { _ in }
    var onMoveText: (Int32, CGSize) -> Void = { _, _ in }
    var onEditText: (Int32) -> Void = { _ in }
    /// The runs of text on this page. The highlighter snaps to these; without
    /// them a sweep across two lines paints one rectangle the height of the drag.
    var segments: [TextSegment] = []
    /// The text selected on this page, if the selection is on this page at all.
    var selection: PageTextSelection?
    var onSelectWord: (CGPoint) -> Void = { _ in }
    var onMoveSelectionHandle: (Bool, CGPoint) -> Void = { _, _ in }
    var onClearSelection: () -> Void = {}
    /// The drawn mark in hand on this page, by its position in `committed`.
    var selectedMark: Int?
    var onSelectMark: (Int?) -> Void = { _ in }
    /// The mark was dragged: by how much, in page points, and whether the finger
    /// has lifted. Only the last one is committed.
    var onMoveMark: (Int, CGSize, Bool) -> Void = { _, _, _ in }
    var onHighlightMissed: () -> Void = {}
    /// A drag that reads as a swipe at the document, made with a tool armed.
    ///
    /// Raised from here because here is the only thing that sees it: with a tool
    /// held the list's own scrolling is off, so the gesture never reaches the
    /// scroll view and nothing else can tell the reader why the page stayed put.
    var onScrollBlocked: () -> Void = {}
    /// Two fingers are on the page: this is a scroll, and whatever the first
    /// finger began is not a mark.
    var twoFingersDown: Bool = false
    /// A redraw token. Referenced inside the `Canvas` closure on purpose: a
    /// closure whose captures have not changed is reused, and an undo left the
    /// removed mark on screen until a zoom forced a repaint.
    var annotationRevision: Int = 0
    /// A note was put down here and needs its words.
    var onRequestNote: (CGPoint) -> Void = { _ in }
    /// An existing note was tapped.
    var onOpenNote: (Int) -> Void = { _ in }
    /// Two fingers are down somewhere: the caption drag stands down for the
    /// pinch. Carried in the environment rather than as another argument — the
    /// two views that build this are already at the type-checker's limit.
    @Environment(\.isPinching) private var pinching
    /// Where the eraser is, for its cursor ring.
    @State private var eraserAt: CGPoint?

    /// The path under the finger, in page points.
    @State private var trace: [CGPoint] = []
    /// The caption being dragged, and how far it has come.
    @State private var movingId: Int32?
    /// How far the drawn mark in hand has been carried, in page points. Overlay
    /// only until the finger lifts — see `onMoveMark`.
    @State private var markShift: CGSize = .zero
    @State private var moveShift: CGSize = .zero

    private var scale: CGFloat { mapping.scale }

    /// Did the finger stay put? Measured as the furthest the trace ever wandered
    /// from where it went down, in page points, against a fixed screen reach.
    /// Was that a swipe at the document rather than a mark?
    ///
    /// Measured on the **net** travel, start to finish, not on the length of the
    /// path: a scribble wanders a long way and ends where it began, and it is the
    /// finger that went somewhere, quickly, that was asking the page to move.
    ///
    /// Three tests, each keeping out a different honest stroke. The reach keeps
    /// out the short deliberate one, which is nearly every stroke. The vertical
    /// dominance keeps out the long one this reader makes most — a highlighter
    /// sweep runs along the words and is almost all sideways, while the pages
    /// only ever go up and down. The lift speed keeps out the case the reach
    /// alone gets wrong: a line or a bracket drawn down a margin is long,
    /// vertical, and finished with the finger at rest.
    private func isSwipeAtDocument(_ value: DragGesture.Value) -> Bool {
        guard settings.tool != .none else { return false }
        // The note tool answers any drag by putting its sheet up, and the sheet
        // would cover the answer.
        if settings.tool == .note { return false }
        // A drag that began on a caption belongs to the layer above, which is
        // carrying the words under the finger — not a scroll that failed.
        if settings.tool.writesText, let down = trace.first,
           captionId(atPagePoint: down) != nil { return false }

        let travel = value.translation
        return abs(travel.height) >= AnnotationMetrics.swipeReach
            && abs(travel.height) >= abs(travel.width) * 2
            && abs(value.velocity.height) >= AnnotationMetrics.swipeLift
    }

    private func isTap(from down: CGPoint) -> Bool {
        let travel = trace.dropFirst().map { hypot($0.x - down.x, $0.y - down.y) }.max() ?? 0
        return travel <= AnnotationMetrics.tapSlop / max(scale, 0.0001)
    }

    var body: some View {
        Canvas { context, _ in
            _ = annotationRevision
            // Under the marks: a highlight made from a selection should read as
            // having been laid over the words, not under the band that chose them.
            drawTextSelection(&context)
            for (index, mark) in committed.enumerated() {
                // The one in hand is drawn where the finger has taken it, not
                // where it still officially sits — the same trick captions use.
                if index == selectedMark, markShift != .zero {
                    draw(mark.movedBy(markShift), in: &context)
                    outline(mark.movedBy(markShift), in: &context)
                    continue
                }
                if index == selectedMark { outline(mark, in: &context) }
                // The one being dragged is drawn where the finger has taken it,
                // not where it still officially sits.
                if case .text(let caption) = mark, caption.id == movingId {
                    var shifted = caption
                    shifted.path = shifted.path.map {
                        CGPoint(x: $0.x + moveShift.width, y: $0.y + moveShift.height)
                    }
                    draw(.text(shifted), in: &context)
                } else {
                    draw(mark, in: &context)
                }
                if case .text(let caption) = mark, caption.id == selectedText {
                    drawSelection(around: caption, in: &context)
                }
            }
            if settings.tool.marks, trace.count >= 1, let preview = build() {
                draw(preview, in: &context)
            }

            // The eraser's cursor, drawn last and over everything: it is a cursor,
            // not a mark, and it has to be legible against whatever it is about
            // to rub out.
            if settings.tool == .eraser, let at = eraserAt {
                let radius = eraserTouchRadius * scale
                context.stroke(
                    Path(ellipseIn: CGRect(x: at.x - radius, y: at.y - radius,
                                           width: radius * 2, height: radius * 2)),
                    with: .color(.black.opacity(0.35)),
                    style: StrokeStyle(lineWidth: 2))
            }
        }
        // **Never hit-testable as a whole.** This layer sits over the page, and
        // a `contentShape` on it swallows every touch beneath — which is what
        // stopped the magnified page zooming at all: the pinch belongs to the
        // view underneath and never received a finger.
        //
        // Only two things here take input, and each limits itself: the caption
        // drag, which is hit-testable only where a caption actually is, and the
        // tool surface, which exists only while a tool is held.
        .allowsHitTesting(false)
        // Underneath the caption layer, and that order matters: overlays stack
        // in the order they are written, so with this one written last it sat
        // **over** the captions and took every drag meant for them. A caption
        // could be tapped but never moved, and the drag it swallowed was read
        // as a placement instead.
        .overlay {
            // Every tool that marks a page — but **not** the snapshot, whose drag
            // belongs to the capture overlay above the whole list. A capture
            // routinely spans two pages, and this surface is per page: armed here
            // it swallowed the drag and quietly accumulated a freehand path that
            // nothing ever read.
            if settings.tool != .none, settings.tool != .snapshot {
                Color.clear
                    .contentShape(Rectangle())
                    .gesture(drawGesture)
            }
        }
        // Putting a caption down when nothing is held.
        //
        // A tap, never a drag: the surface spans the page, and a drag gesture
        // here would stop the document scrolling everywhere a caption happens to
        // be selected. It exists only while one is held, so the rest of the time
        // there is nothing over the page at all.
        .overlay {
            if settings.tool == .none, selectedText != nil {
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture { onSelectText(nil) }
            }
        }
        // With nothing held, and with something that writes words held — but not
        // while a pen is in hand, where a tap has to be a mark.
        //
        // The cost of answering with no tool armed is that a one-finger drag
        // beginning **on** a caption moves it instead of scrolling the document.
        // Everywhere else still scrolls: this layer is hit-testable only where a
        // caption actually is.
        // Selecting text needs no tool, the way it needs none on Android: a long
        // press anywhere on the words does it. Below the caption layer, so a
        // press that lands on a caption still belongs to the caption.
        .overlay {
            if settings.tool == .none {
                TextSelectLayer(
                    onLongPress: { onSelectWord(toPage($0)) },
                    onTap: { at in
                        // A drawn mark under the finger goes in hand; bare page
                        // puts down whatever was. The caption layer above this one
                        // is hit-testable only where a caption is, so a tap on
                        // words never reaches here.
                        onSelectMark(markIndex(atPagePoint: toPage(at)))
                    },
                    heldMark: selectedMark.flatMap { index in
                        committed.indices.contains(index)
                            ? committed[index].bounds.map { box in
                                let corner = mapping.toScreen(CGPoint(x: box.minX, y: box.minY))
                                let far = mapping.toScreen(CGPoint(x: box.maxX, y: box.maxY))
                                return CGRect(x: corner.x, y: corner.y,
                                              width: far.x - corner.x,
                                              height: far.y - corner.y)
                              }
                            : nil
                    },
                    onMoveMark: { offset, done in
                        let delta = CGSize(width: offset.width / scale,
                                           height: offset.height / scale)
                        markShift = done ? .zero : delta
                        if let index = selectedMark { onMoveMark(index, delta, done) }
                    },
                    // Only the page the selection is actually on.
                    //
                    // `selection` is the reader's one selection, handed to every
                    // page in the list. Without this check each of them armed its
                    // handle drag and its tap-to-clear against handle positions
                    // belonging to a different page — so a drag on any page took
                    // hold of a phantom handle, and with the drag now claiming any
                    // touch (below) that would have stopped the list scrolling
                    // everywhere at once.
                    handles: selection.flatMap { found in
                        found.pageIndex == pageIndex
                            ? (start: mapping.toScreen(found.startHandle),
                               end: mapping.toScreen(found.endHandle))
                            : nil
                    },
                    onMoveHandle: { isStart, at in
                        onMoveSelectionHandle(isStart, toPage(at))
                    },
                    onTapAway: onClearSelection,
                    selectionActive: selection != nil)
            }
        }
        .overlay {
            if settings.tool == .none || settings.tool.writesText {
            CaptionMoveLayer(
                captionAt: { location in captionId(atViewPoint: location) },
                selected: selectedText,
                pinching: pinching,
                onMove: { id, offset in
                    // One entry per frame of the drag. The gaps between these are
                    // the answer to "why does this feel heavy": at 16ms the drag is
                    // keeping up, at 100ms it is not, and `PAGE_READABLE` landing
                    // in between says the page is re-rasterising mid-drag when it
                    // has no business doing so.
                    SessionRecorder.shared.record("TOOL_GESTURE",
                        String(format: "caption move id=%d by=%.0f,%.0f scale=%.2f",
                               id, offset.width, offset.height, scale))
                    movingId = id
                    moveShift = CGSize(width: offset.width / scale, height: offset.height / scale)
                },
                onFinish: { id, offset in
                    movingId = nil
                    moveShift = .zero
                    // Only if the finger actually went somewhere. A press that
                    // did not move is a tap — the tap handler has it, and
                    // committing a zero move here would leave an undo step that
                    // undoes nothing.
                    guard offset != .zero else { return }
                    onMoveText(id, CGSize(width: offset.width / scale,
                                          height: offset.height / scale))
                },
                onCancel: { _ in
                    movingId = nil
                    moveShift = .zero
                },
                onTap: { id in onSelectText(id) },
                onDoubleTap: { id in onEditText(id) })
            }
        }
    }

    private var drawGesture: some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                guard settings.tool != .none else { return }
                // A second finger turns this into a scroll. The stroke is thrown
                // away rather than merely stopped: by the time the second finger
                // lands the first has already drawn a line, and leaving it would
                // put a mark down the page every time the reader scrolled with a
                // tool held — which is the very gesture the scroll hint sends
                // people to.
                if twoFingersDown {
                    if !trace.isEmpty { trace = [] }
                    eraserAt = nil
                    return
                }
                let point = toPage(value.location)

                if settings.tool == .eraser {
                    // One sweep, one undo step — bracketed here because this is
                    // the only place that knows where the sweep began.
                    if eraserAt == nil { onEraseStart() }
                    eraserAt = value.location
                    onErase(point, tolerance)
                    return
                }
                if settings.tool == .note {
                    trace = [point]
                } else if settings.tool.isDragged {
                    // Two corners, not a path: the shape is defined by where the
                    // drag started and where it is now.
                    trace = [toPage(value.startLocation), point]
                } else {
                    trace.append(point)
                }
            }
            .onEnded { value in
                // Nothing was drawn, so there is nothing to commit and nothing to
                // say about scrolling either.
                guard !twoFingersDown else {
                    trace = []
                    eraserAt = nil
                    return
                }
                // Asked before every tool's own ending — before the eraser's,
                // which returns immediately below, and before the signature's,
                // which ends in no branch at all. The mark is still made; this
                // only says why the document did not move under it.
                if isSwipeAtDocument(value) { onScrollBlocked() }
                defer { trace = []; eraserAt = nil }
                if settings.tool == .eraser {
                    onEraseEnd()
                    return
                }

                // The three-way rule for the text tools. Tapping an existing
                // caption takes it in hand; tapping empty page with one in hand
                // only puts it down. Placing a second caption therefore takes two
                // taps, which is what stops one being stamped on top of another.
                // A note is placed, then written. Committing it empty is how a
                // note came to be unreadable and un-editable the moment it
                // existed.
                if settings.tool == .note, let down = trace.first {
                    if let existing = noteIndex(atPagePoint: down) {
                        onOpenNote(existing)
                    } else {
                        onRequestNote(down)
                    }
                    return
                }

                if settings.tool.writesText, let down = trace.first {
                    // Placing, selecting and putting down all come from a **tap**,
                    // and nothing else. A drag belongs to the caption layer above,
                    // which moves the words under the finger.
                    //
                    // Both layers see the same touch — the caption layer's
                    // recogniser and this gesture each get it — so without this
                    // guard one drag on a caption moved it *and* stamped a second
                    // caption at the point the finger went down.
                    guard isTap(from: down) else { return }

                    if let hit = captionId(atPagePoint: down) {
                        onSelectText(hit)
                        return
                    }
                    // A tap on empty page puts the held caption down rather than
                    // placing another. Placing a second therefore takes two taps,
                    // which is what stops one being stamped on top of another.
                    if selectedText != nil {
                        onSelectText(nil)
                        return
                    }
                    // The down point, and only that. The line the words run along
                    // is measured from the font's own metrics — a drawn baseline
                    // has a length unrelated to the text, so anything longer is
                    // truncated against it.
                    onPlaceText(down, nil)
                    return
                }
                guard settings.tool.marks, let annotation = build() else { return }
                onCommit(annotation)
            }
    }

    /// The dashed box around a drawn mark in hand.
    ///
    /// The same notation a caption gets, so "this one is chosen" looks the same
    /// whatever kind of mark it is.
    private func outline(_ mark: WireAnnotation, in context: inout GraphicsContext) {
        guard let box = mark.bounds else { return }
        let margin: CGFloat = 6 / max(scale, 0.0001)
        let corner = mapping.toScreen(CGPoint(x: box.minX - margin, y: box.minY - margin))
        let far = mapping.toScreen(CGPoint(x: box.maxX + margin, y: box.maxY + margin))
        let rect = CGRect(x: corner.x, y: corner.y,
                          width: far.x - corner.x, height: far.y - corner.y)
        context.stroke(Path(roundedRect: rect, cornerRadius: 4),
                       with: .color(Color(hex: 0xF2A93B)),
                       style: StrokeStyle(lineWidth: 1.5, dash: [5, 4]))
    }

    /// The bands over the selected words, and a handle at each end.
    ///
    /// One band per line rather than one per character — `PageCharacters` merges
    /// them — which is what makes a selection read as a stripe over the text
    /// instead of a row of little boxes, and is far less to draw on a page where
    /// a selection can run to thousands of characters.
    private func drawTextSelection(_ context: inout GraphicsContext) {
        guard let selection, selection.pageIndex == pageIndex else { return }

        // The system's own selection blue: selecting text is a system idiom, and
        // a reader inventing its own colour for it makes the gesture look like
        // something else.
        let tint = Color(uiColor: .tintColor)
        for rect in selection.rects {
            let corner = mapping.toScreen(CGPoint(x: rect.minX, y: rect.minY))
            let far = mapping.toScreen(CGPoint(x: rect.maxX, y: rect.maxY))
            let band = CGRect(x: corner.x, y: corner.y,
                              width: far.x - corner.x, height: far.y - corner.y)
            context.fill(Path(roundedRect: band, cornerRadius: 2), with: .color(tint.opacity(0.28)))
        }

        // Below the baseline, the way a text cursor's grip sits: a handle drawn
        // over the words hides the very letters being aimed at.
        for point in [selection.startHandle, selection.endHandle] {
            let at = mapping.toScreen(point)
            let dot = CGRect(x: at.x - 6, y: at.y - 1, width: 12, height: 12)
            context.fill(Path(ellipseIn: dot), with: .color(tint))
        }
    }

    private func toPage(_ location: CGPoint) -> CGPoint {
        // Clamped on the way in. What is stored has to be on the page — the
        // draw-time clip only hides the half you can see.
        mapping.clampToPage(mapping.toPage(location))
    }

    /// How far off a mark still counts as touching it, in page points.
    private var tolerance: CGFloat { scale > 0 ? eraserTouchRadius / scale : eraserTouchRadius }

    private func captionId(atViewPoint location: CGPoint) -> Int32? {
        captionId(atPagePoint: toPage(location))
    }

    /// The topmost caption at this point — last drawn is the one on top, and the
    /// one the finger is pointing at.
    /// The topmost drawn mark at this point, by its position in the committed
    /// list, or nil if the tap landed on bare page.
    ///
    /// Ink only. `movedBy` moves every kind — see `PagifyEdit` — but a highlight
    /// belongs to the words it covers, and one dragged off them is wrong in the
    /// file as well as on screen. A note has its own tap, which opens it. Offering
    /// a case later is one line here.
    private func markIndex(atPagePoint point: CGPoint) -> Int? {
        for index in committed.indices.reversed() {
            if case .ink = committed[index], committed[index].isHitBy(point, tolerance: tolerance) {
                return index
            }
        }
        return nil
    }

    private func captionId(atPagePoint point: CGPoint) -> Int32? {
        for mark in committed.reversed() {
            if case .text(let caption) = mark,
               mark.isHitBy(point, tolerance: tolerance) {
                return caption.id
            }
        }
        return nil
    }

    /// The topmost note at this point, by its position in the committed list.
    private func noteIndex(atPagePoint point: CGPoint) -> Int? {
        for (index, mark) in committed.enumerated().reversed() {
            if case .note = mark, mark.isHitBy(point, tolerance: tolerance) { return index }
        }
        return nil
    }

    /// A dashed box round the caption in hand. Nothing else — no handles, no
    /// wash, no delete badge.
    private func drawSelection(around caption: TextMark, in context: inout GraphicsContext) {
        var box = caption.textFrameBounds()
        if caption.frame == .none { box = caption.textBlockBounds() }
        let gap: CGFloat = 6 / max(scale, 0.001)
        let corner = mapping.toScreen(CGPoint(x: box.left - gap, y: box.top - gap))
        let rect = CGRect(x: corner.x, y: corner.y,
                          width: (box.right - box.left + gap * 2) * scale,
                          height: (box.bottom - box.top + gap * 2) * scale)
        let width = min(max(caption.size * 0.09 * scale, 1.5), 6)
        context.stroke(Path(rect), with: .color(Color(hex: 0xF2A93B)),
                       style: StrokeStyle(lineWidth: width,
                                          dash: [width * 3, width * 2.5]))
    }

    /// The mark the current trace represents, or nil if it is not one yet.
    private func build() -> WireAnnotation? {
        let colour = settings.penColor
        let width = settings.strokeWidth

        switch settings.tool {
        case .highlight:
            guard trace.count >= 2 else { return nil }
            // One rect per covered run, not one rect the height of the drag. A
            // selection spanning three lines is still **one** annotation, so
            // erasing it takes one action rather than three.
            let rects = TextSelection.rectsBetween(segments,
                                                   anchor: trace[0],
                                                   focus: trace[trace.count - 1])
            guard !rects.isEmpty else { return nil }
            return .highlight(rects: rects, color: colour)

        case .pen:
            guard trace.count >= 2 else { return nil }
            return .ink(strokes: dashed(trace, style: settings.style, width: width),
                        color: colour, width: width)

        case .cloud:
            let outline = cloudOutline(trace, width: width)
            guard outline.count >= 2 else { return nil }
            return .ink(strokes: dashed(outline, style: settings.style, width: width),
                        color: colour, width: width)

        case .curve:
            guard trace.count >= 3 else { return nil }
            return .ink(strokes: dashed(curveThrough(trace), style: settings.style, width: width),
                        color: colour, width: width)

        case .curvedArrow:
            guard trace.count >= 3 else { return nil }
            let strokes = curvedArrowStrokes(trace, width: width, style: settings.style)
            guard !strokes.isEmpty else { return nil }
            return .ink(strokes: strokes, color: colour, width: width)

        case .line, .arrow, .rectangle, .ellipse:
            guard trace.count == 2 else { return nil }
            let strokes = shapeStrokes(tool: settings.tool, start: trace[0], end: trace[1],
                                       style: settings.style, width: width)
            guard !strokes.isEmpty else { return nil }
            return .ink(strokes: strokes, color: colour, width: width)

        case .signature:
            // Deliberately ink: a signature *is* a set of strokes, and giving it
            // its own annotation type would mean a second thing to read back,
            // render and erase for no difference anyone could see.
            guard trace.count >= 2 else { return nil }
            return .ink(strokes: [trace], color: colour, width: AnnotationMetrics.signatureWidth)

        // A caption is placed, then typed: the gesture only says where. The
        // commit happens when the words exist.
        case .text, .curvedText, .cloudText, .boxText, .ellipseText:
            return nil

        // A note is placed and then written, so the gesture commits nothing —
        // `onRequestNote` asks for the words and the model commits when it has
        // them. The rest make no mark at all.
        case .note, .none, .eraser, .snapshot:
            return nil
        }
    }

    /// The mark as it will be committed, drawn at screen scale.
    ///
    /// Built from the same `build()` the commit uses rather than from a separate
    /// preview path, so what is on screen under the finger and what lands in the
    /// file cannot disagree.
    private func draw(_ annotation: WireAnnotation, in context: inout GraphicsContext) {
        let colour: Color
        switch annotation {
        case .highlight(_, let c), .ink(_, let c, _), .note(_, _, let c):
            colour = Color(c.cgColor)
        case .text(let mark):
            colour = Color(mark.color.cgColor)
        }

        switch annotation {
        case .highlight(let rects, let stored):
            // A fixed wash, overriding whatever alpha the mark was stored with.
            // The constants are opaque on purpose — an alpha baked into the
            // stored colour would come back paler on every re-save.
            let wash = Color(stored.withAlpha(255).cgColor)
                .opacity(AnnotationColors.highlightAlpha)
            for rect in rects {
                let r = CGRect(x: mapping.toScreen(CGPoint(x: rect.left, y: rect.top)).x,
                               y: mapping.toScreen(CGPoint(x: rect.left, y: rect.top)).y,
                               width: (rect.right - rect.left) * scale,
                               height: (rect.bottom - rect.top) * scale)
                context.fill(Path(r), with: .color(wash))
            }

        case .ink(let strokes, _, let width):
            // Freehand is smoothed, shapes are not. A pen stroke is a list of
            // touch samples and joining them with straight segments shows every
            // one of them as a corner; a rectangle's points *are* corners, and
            // curving through them would round the box off.
            let smooth = settings.tool.tracesPath && settings.tool != .cloud
            for stroke in strokes where stroke.count >= 2 {
                let scaled = stroke.map { mapping.toScreen($0) }
                var path = Path()
                path.move(to: scaled[0])
                if smooth {
                    for index in 1..<scaled.count {
                        let previous = scaled[index - 1]
                        let current = scaled[index]
                        path.addQuadCurve(
                            to: CGPoint(x: (previous.x + current.x) / 2,
                                        y: (previous.y + current.y) / 2),
                            control: previous)
                    }
                    path.addLine(to: scaled[scaled.count - 1])
                } else {
                    for point in scaled.dropFirst() { path.addLine(to: point) }
                }
                context.stroke(path, with: .color(colour),
                               style: StrokeStyle(lineWidth: width * scale,
                                                  lineCap: .round, lineJoin: .round))
            }

        case .note(let rect, _, _):
            // Three layers, in order. The outline exists because a 7pt yellow dot
            // on white was reported as the note not having been added at all.
            let centre = mapping.toScreen(CGPoint(x: (rect.left + rect.right) / 2,
                                                  y: (rect.top + rect.bottom) / 2))
            let radius = AnnotationMetrics.noteMarkerRadius * scale
            let disc = CGRect(x: centre.x - radius, y: centre.y - radius,
                              width: radius * 2, height: radius * 2)
            context.fill(Path(ellipseIn: disc), with: .color(colour))
            context.stroke(Path(ellipseIn: disc), with: .color(.black.opacity(0.65)),
                           style: StrokeStyle(lineWidth: 1.2 * scale))
            let pip = radius * 0.32
            context.fill(Path(ellipseIn: CGRect(x: centre.x - pip, y: centre.y - pip,
                                                width: pip * 2, height: pip * 2)),
                         with: .color(.black.opacity(0.65)))

        case .text(let mark):
            // The ring first, then the words — so the letters sit on top of the
            // frame rather than being covered by it.
            let ring = mark.textFrameOutline()
            if ring.count >= 2 {
                var path = Path()
                path.move(to: mapping.toScreen(ring[0]))
                for point in ring.dropFirst() {
                    path.addLine(to: mapping.toScreen(point))
                }
                context.stroke(path, with: .color(colour),
                               style: StrokeStyle(lineWidth: mark.size * textFrameStroke * scale,
                                                  lineCap: .round, lineJoin: .round))
            }

            // Drawn glyph by glyph from the same layout the commit sends, so what
            // is on screen and what lands in the file cannot disagree — and in the
            // mark's **own** face, because a caption previewed in the system font
            // and written in Naskh is two different captions.
            let face = mark.font.uiFont(size: mark.size * scale)
            let fill = UIColor(colour).cgColor
            for glyph in mark.layOutBlock() {
                let origin = mapping.toScreen(glyph.origin)

                // By glyph **id**, not by the characters it came from.
                //
                // The shaper has already decided which form each letter takes:
                // Arabic letters join, and the joined shape is a glyph with no
                // character of its own. Handing the cluster's characters back to
                // `Text` throws that away and asks CoreText to shape one letter
                // with no neighbours, which can only produce the isolated form —
                // so a caption that read correctly while being typed came out on
                // the page as a row of disconnected letters. The id is the form
                // that was chosen, and the file gets that same id.
                if glyph.id != 0, mark.font.asset != nil {
                    context.drawLayer { layer in
                        layer.translateBy(x: origin.x, y: origin.y)
                        // Curved captions lean with the baseline.
                        if glyph.radians != 0 { layer.rotate(by: .radians(glyph.radians)) }
                        layer.withCGContext { cg in
                            // CoreText lays glyphs out with y running up; this
                            // context has it running down. The flip is about the
                            // origin, which *is* the baseline — so no ascent
                            // fudge is needed, unlike the `Text` path below.
                            cg.scaleBy(x: 1, y: -1)
                            cg.setFillColor(fill)
                            var id = CGGlyph(truncatingIfNeeded: glyph.id)
                            var at = CGPoint.zero
                            CTFontDrawGlyphs(face as CTFont, &id, &at, 1, cg)
                        }
                    }
                    continue
                }

                // A standard-14 face has no embedded file and no glyph ids; its
                // scripts are the ones where a character is its own glyph anyway.
                var resolved = context.resolve(
                    Text(glyph.text).font(Font(face)).foregroundColor(colour))
                resolved.shading = .color(colour)
                context.drawLayer { layer in
                    layer.translateBy(x: origin.x, y: origin.y)
                    if glyph.radians != 0 { layer.rotate(by: .radians(glyph.radians)) }
                    // The origin is the *baseline*, so the run is lifted by the
                    // face's own ascent rather than by a guessed fraction of the
                    // point size.
                    layer.draw(resolved, at: CGPoint(x: 0, y: -face.ascender), anchor: .topLeading)
                }
            }
        }
    }
}


// ------------------------------------------------------- the pinch signal --

/// Whether two fingers are down on the reader.
///
/// A gesture recogniser is only handed the touches that land on its **own** view,
/// and the second finger of a pinch almost always lands on bare page — so the
/// caption drag cannot tell a pinch from a one-finger drag by itself, and goes on
/// carrying the words around while the other hand is trying to resize them.
private struct IsPinchingKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var isPinching: Bool {
        get { self[IsPinchingKey.self] }
        set { self[IsPinchingKey.self] = newValue }
    }
}
