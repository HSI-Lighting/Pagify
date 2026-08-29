import SwiftUI

/// A single page, magnified. Android's `ui/reader/ZoomedPage.kt`.
///
/// Zooming deliberately leaves the continuous list behind and scopes the view to
/// one page: panning a magnified page should never wander into its neighbours,
/// which is disorienting and loses your place.
///
/// ## Why this does its own transform instead of using a scroll view
///
/// The position of the content is held here as an explicit offset and applied
/// when it is drawn. That is what makes pinch anchoring *exact*: the new offset
/// is computed and applied in the same frame as the gesture event.
///
/// Driving layout width from the zoom and letting a scroll view position it
/// cannot do that. The anchoring correction has to wait for a re-measure, a pinch
/// fires dozens of events, and each one restarts and cancels the pending
/// correction — the survivor applies a stale ratio against an offset that has
/// already moved, so the zoom drifts away from the fingers and walks the reader
/// back to the top of the document.
///
/// Sharpness is kept separately: the page is *rasterised* at `committedScale`,
/// which catches up once a gesture settles.
struct ZoomedPageView: View {
    let document: PagifyDocument
    let pageIndex: Int
    let initialZoom: CGFloat
    /// Where to centre when the view opens, as a 0…1 fraction of the page.
    let initialFocus: CGPoint?
    /// The width, in points, that the list draws this page at.
    ///
    /// This is what scale 1.0 has to mean. This view replaces the whole row
    /// including the rail, so its own viewport is *wider* than the reader area
    /// the page was occupying — measuring the base against the viewport makes
    /// entering zoom silently enlarge the page by the width of the rail before
    /// any gesture applies.
    let basePageWidth: CGFloat
    /// Whatever the list last drew for this page, so the first frame is not blank.
    let initialImage: CGImage?
    let onZoomSettled: (CGFloat) -> Void
    /// Move to the next page (+1) or the previous (−1), staying magnified.
    /// Returns false when there is nowhere to go, which springs the pull back.
    let onTurnPage: (Int) -> Bool
    let onExit: () -> Void
    /// The marks already on this page, and the state of the tool ribbon.
    ///
    /// Annotating has to work here too. This is a separate render path from the
    /// list, and drawing the page bitmap and nothing else is what made magnifying
    /// a page hide its highlights and leave the pen with no surface to draw on —
    /// every one-finger drag went to the pan handler instead.
    let settings: AnnotationSettings
    let committed: [WireAnnotation]
    let annotationRevision: Int
    /// Bumped whenever the page's own content changes. Captions are drawn **into**
    /// the page, not over it, so without this the magnified view keeps showing the
    /// words where they used to be while the overlay draws them where they are.
    let contentRevision: Int
    let segments: [TextSegment]
    let selectedText: Int32?
    let onCommit: (WireAnnotation) -> Void
    let onErase: (CGPoint) -> Void
    let onEraseStart: () -> Void
    let onEraseEnd: () -> Void
    let onPlaceText: (CGPoint, CGPoint?) -> Void
    let onSelectText: (Int32?) -> Void
    let onMoveText: (Int32, CGSize) -> Void
    let onEditText: (Int32) -> Void
    let onHighlightMissed: () -> Void
    var onScrollBlocked: () -> Void = {}
    let onRequestNote: (CGPoint) -> Void
    let onOpenNote: (Int) -> Void
    /// Two fingers with a caption in hand: that big.
    let onScaleText: (CGFloat) -> Void

    /// Selecting text, magnified.
    ///
    /// Absent until now, and that absence was the whole of "long press does
    /// nothing". The press layer is built on the tool alone — `settings.tool ==
    /// .none` inside `AnnotationCanvas` — so it existed here, the recogniser
    /// fired, and it called the canvas's defaulted no-op. No selection, no bar,
    /// and not even the notice `selectWord` raises for a page with no text layer.
    ///
    /// The list had all four from the start, which is why it passed on a
    /// simulator: on a Mac-sized window a page in the list is big enough to aim a
    /// word at. On a phone it is a thumbnail, so the magnified page is the only
    /// place anyone would try this — and it was the one view never handed them.
    ///
    /// A departure from Android, which has the same hole: `ZoomedPage.kt` calls
    /// `annotationLayer` without any of these, so selecting text in a magnified
    /// page does nothing there either. Fixed on this side because it is where it
    /// was reported; the Kotlin wants the same four.
    var selection: PageTextSelection?
    var onSelectWord: (CGPoint) -> Void = { _ in }
    var onMoveSelectionHandle: (Bool, CGPoint) -> Void = { _, _ in }
    var onClearSelection: () -> Void = {}
    var selectedMark: Int?
    var onSelectMark: (Int?) -> Void = { _ in }
    var onMoveMark: (Int, CGSize, Bool) -> Void = { _, _, _ in }
    /// Where the page is drawn on screen right now, in this view's coordinates.
    ///
    /// The snapshot tool needs it. In the list the reader already knows every
    /// page's rectangle, because it puts them there; here the page is *drawn*
    /// rather than laid out — at a scale and an offset only this view holds — so
    /// nothing outside can work out where a drag landed on the sheet without
    /// being told.
    var onPageFrame: (CGRect) -> Void = { _ in }
    /// Raised while two fingers are down, so this page's caption layer lets go of
    /// the words rather than carrying them along under the pinch.
    @State private var pinching = false
    /// The same signal, told to the **model**.
    ///
    /// Without it the model never learns a pinch is running, so every frame of a
    /// caption resize writes straight into the document — and a caption is page
    /// content, so each write re-rasterises the whole magnified page. A recording
    /// caught that costing 177-324ms per frame on a 3170x4509 raster, arriving
    /// every 150ms: the resize could not keep up because it was re-rendering
    /// fourteen megapixels between one finger movement and the next.
    var onPinching: (Bool) -> Void = { _ in }
    /// Whether this view has drawn at least once.
    @State private var didDraw = false

    @Environment(\.displayScale) private var displayScale
    @Environment(\.colorScheme) private var scheme

    @State private var scale: CGFloat = 1
    @State private var committedScale: CGFloat = 1
    /// Whether the opening zoom and seed bitmap have been taken. Done in a task
    /// rather than an initialiser so the memberwise one stays synthesised — a
    /// hand-written init here has to be updated every time a callback is added,
    /// and forgetting is a compile error in the wrong file.
    @State private var opened = false
    /// Which page the raster on screen belongs to.
    @State private var renderedPage: Int?
    @State private var recentred = false
    @State private var lastPinch: CGFloat = 1
    @State private var offset: CGPoint = .zero
    /// How far the page has been pulled past its own end, in points.
    ///
    /// Signed the way the offset is. It gives a little and springs back, which
    /// says "the page has run out" without a word on screen — and a fresh swipe
    /// that pulls far enough turns to the next page.
    @State private var pull: CGFloat = 0
    /// Which edge to sit at on a page just turned to.
    @State private var landing: Int?

    @State private var image: CGImage?
    @State private var imageScale: CGFloat = 0
    @State private var pageSize: CGSize?

    private static let defaultAspect: CGFloat = 595.0 / 842.0
    /// How long after the last gesture event to re-rasterise at the new scale.
    private static let settleMillis = 180
    /// How much of a drag past the page's end actually moves it. Less than all of
    /// it, so the edge feels like an edge.
    private static let pullDamping: CGFloat = 0.45
    /// As far as the page will give, however hard it is pulled.
    private static let pullLimit: CGFloat = 240
    /// How far it must be pulled for a lift to turn the page. Comfortably short
    /// of the limit, so it turns before the gesture stops meaning anything.
    private static let pullToTurn: CGFloat = 110

    var body: some View {
        GeometryReader { geometry in
            let viewport = geometry.size
            let aspect = pageSize.map { $0.width / $0.height } ?? Self.defaultAspect
            let baseW = basePageWidth > 0 ? basePageWidth : viewport.width
            let baseH = aspect > 0 ? baseW / aspect : viewport.height
            let shown = CGPoint(x: offset.x, y: offset.y + pull)

            Canvas { context, _ in
                guard let image else { return }
                // The page is **drawn**, not laid out, at its magnified size. A
                // view laid out at baseW × scale asks the compositor for a layer
                // that size — at 4x that is past the maximum texture size and the
                // layer silently fails, so the page vanishes the moment the
                // gesture settles. Here the canvas is always viewport-sized and
                // only the destination rectangle grows.
                let destination = CGRect(x: shown.x, y: shown.y,
                                         width: max(baseW * scale, 1),
                                         height: max(baseH * scale, 1))
                context.draw(Image(decorative: image, scale: displayScale)
                    .resizable()
                    .interpolation(.medium), in: destination)
            }
            .frame(width: viewport.width, height: viewport.height)
            .background(PagifyColor.background(scheme))
            .clipped()
            .onChange(of: CGRect(x: shown.x, y: shown.y,
                                 width: max(baseW * scale, 1),
                                 height: max(baseH * scale, 1))) { _, frame in
                onPageFrame(frame)
            }
            .onAppear {
                onPageFrame(CGRect(x: shown.x, y: shown.y,
                                   width: max(baseW * scale, 1),
                                   height: max(baseH * scale, 1)))
            }
            .contentShape(Rectangle())
            // The UIKit pinch, not SwiftUI's.
            //
            // `MagnifyGesture` never fired here at all: the two-finger pan layer
            // over this view claims the touches first, and SwiftUI's gesture then
            // never sees a second finger. So zooming the magnified page, and
            // resizing a caption on it, both did nothing.
            //
            // This host reads the *same* events as the pan layer — both re-home
            // their recogniser onto the superview and allow simultaneous
            // recognition — and classifies the gesture the way Android does:
            // whichever of finger separation or midpoint travel passes the slop
            // first decides what it is, and that holds until the fingers lift. It
            // also reports the **live centroid**, so the page scales about the
            // point between the fingers rather than about where the pinch began.
            .overlay {
                PinchToZoomLayer(
                    onZoomBy: { factor, centroid in
                        pinching = true
                        onPinching(true)
                        guard selectedText == nil else {
                            onScaleText(factor)
                            return
                        }
                        zoomAbout(factor, focus: centroid,
                                  viewport: viewport, baseW: baseW, baseH: baseH)
                    },
                    onGestureEnd: {
                        pinching = false
                        onPinching(false)
                        // Only a pinch that was about the *page* can leave it. A
                        // caption resized at fit width would otherwise throw the
                        // reader back to the list the moment the fingers lift.
                        guard selectedText == nil else { return }
                        if scale <= Zoom.fitWidth + 0.01 { onExit() }
                    })
            }
            // Two fingers always pan, for the same reason as in the list: the
            // pinch claims every two-finger event, so nothing else would ever
            // receive one.
            .overlay {
                TwoFingerPanLayer { delta in
                    guard selectedText == nil else { return }
                    offset = clamp(CGPoint(x: offset.x + delta.width, y: offset.y + delta.height),
                                   atScale: scale, viewport: viewport,
                                   baseW: baseW, baseH: baseH)
                }
            }
            // With a tool live, one finger belongs to the tool — exactly as in the
            // list. Leaving the pan on one finger is what makes the pen look
            // disabled here: the drag is consumed before the drawing layer sees it.
            //
            // Attached **after** the pinch, and that order is the whole point.
            //
            // `including:` masks the gestures of the view it is applied to. With a
            // tool armed the mask is `.subviews`, which switches off this view's
            // own gestures — and when this sat above the pinch, it switched the
            // pinch off with it. Arming any tool killed zooming and caption
            // resizing outright in the magnified page. Applied out here, the pinch
            // is a subview gesture: `.subviews` turns off the page pan and leaves
            // everything else alone, which is what it always meant.
            // Words selected stand the page pan down as well, for the same
            // reason a tool does: the finger has something else to mean, and a
            // page sliding under a selection being adjusted is the drag going to
            // the wrong place.
            .gesture(pan(viewport: viewport, baseW: baseW, baseH: baseH),
                     including: settings.tool == .none && selection == nil
                        && selectedMark == nil ? .all : .subviews)
            .environment(\.isPinching, pinching)
            .simultaneousGesture(SpatialTapGesture(count: 2).onEnded { tap in
                // Stands down while a caption is in hand, alongside the pinch and
                // the two-finger pan: with one held, two taps on it mean "let me
                // rewrite this", and zooming the page out from under that reads as
                // the app having ignored you.
                guard selectedText == nil else { return }
                // About the tapped point, not the middle. Reported immediately
                // rather than after the settle delay, so the pinned page is
                // released without a visible lag.
                let focus = tap.location
                if scale > Zoom.fitWidth + 0.01 {
                    zoomAbout(Zoom.fitWidth / scale, focus: focus,
                              viewport: viewport, baseW: baseW, baseH: baseH)
                    onExit()
                } else {
                    zoomAbout(Zoom.doubleTap / scale, focus: focus,
                              viewport: viewport, baseW: baseW, baseH: baseH)
                }
                committedScale = scale
                onZoomSettled(scale)
            })
            // Innermost, so a one-finger drag reaches the tool before anything
            // else can claim it, and so marks are drawn over the page.
            .overlay {
                if let size = pageSize {
                    AnnotationCanvas(
                        pageIndex: pageIndex,
                        // The page is drawn translated, so the layer has to be
                        // told where its top-left corner actually is.
                        mapping: PageMapping(scale: size.width > 0 ? baseW * scale / size.width : 0,
                                             origin: shown,
                                             pageWidthPoints: size.width,
                                             pageHeightPoints: size.height),
                        settings: settings,
                        onCommit: onCommit,
                        onErase: onErase,
                        onEraseStart: onEraseStart,
                        onEraseEnd: onEraseEnd,
                        onPlaceText: onPlaceText,
                        committed: committed,
                        selectedText: selectedText,
                        onSelectText: onSelectText,
                        onMoveText: onMoveText,
                        onEditText: onEditText,
                        segments: segments,
                        selection: selection,
                        onSelectWord: onSelectWord,
                        onMoveSelectionHandle: onMoveSelectionHandle,
                        onClearSelection: onClearSelection,
                        selectedMark: selectedMark,
                        onSelectMark: onSelectMark,
                        onMoveMark: onMoveMark,
                        onHighlightMissed: onHighlightMissed,
                        onScrollBlocked: onScrollBlocked,
                        annotationRevision: annotationRevision,
                        onRequestNote: onRequestNote,
                        onOpenNote: onOpenNote)
                }
            }
            .task(id: "\(pageIndex)-\(contentRevision)") {
                // Skipped on the first pass: `.task(id: pageIndex)` below already
                // draws it, and rasterising twice on entry is a visible stutter.
                guard didDraw else { return }
                await rasterise(baseW: baseW, force: true)
            }
            .task(id: pageIndex) {
                // The raster belongs to the page it was drawn for. Keeping it
                // across a turn leaves the previous page on screen indefinitely,
                // because the scale guard below then refuses to redraw.
                if renderedPage != pageIndex {
                    renderedPage = pageIndex
                    image = nil
                    imageScale = 0
                }
                if !opened {
                    SessionRecorder.shared.record("PAGE_ENTER",
                       String(format: "pinned page=%d zoom=%.2f base=%.0f",
                              pageIndex, initialZoom, basePageWidth))
                    scale = initialZoom
                    committedScale = initialZoom
                    image = initialImage
                    opened = true
                }
                pageSize = try? document.pageSize(pageIndex)
                await rasterise(baseW: baseW)
            }
            // Open centred on whatever the entering gesture was aimed at.
            .task(id: "\(baseW)-\(baseH)") {
                // Keyed without the page: re-applying the entering gesture's
                // focus on every turn fights the rule that puts a page turned to
                // at the edge the reader arrived from.
                guard let focus = initialFocus, !recentred else { return }
                recentred = true
                offset = clamp(CGPoint(x: viewport.width / 2 - focus.x * baseW * scale,
                                       y: viewport.height / 2 - focus.y * baseH * scale),
                               atScale: scale, viewport: viewport, baseW: baseW, baseH: baseH)
            }
            // Held back until the gesture stops, so a pinch does not rasterise
            // the page at every intermediate size.
            .task(id: scale) {
                try? await Task.sleep(for: .milliseconds(Self.settleMillis))
                guard !Task.isCancelled else { return }
                SessionRecorder.shared.record("ZOOM_SETTLED", String(format: "page=%d scale=%.2f", pageIndex, scale))
                committedScale = scale
                onZoomSettled(scale)
                await rasterise(baseW: baseW)
            }
            // A page just turned to sits at the edge the reader arrived from: the
            // top going forward, the bottom going back, and always at the same
            // horizontal position, so reading down one column carries on in the
            // same column of the next page.
            .task(id: "\(pageIndex)-\(landing ?? 0)") {
                guard let towards = landing else { return }
                offset = clamp(CGPoint(x: offset.x,
                                       y: towards > 0 ? 0 : viewport.height - baseH * scale),
                               atScale: scale, viewport: viewport, baseW: baseW, baseH: baseH)
            }
        }
        .ignoresSafeArea(edges: .horizontal)
    }

    // ----------------------------------------------------------- geometry --

    /// Keep the content covering the viewport, and centred on any axis where it
    /// is smaller. Without this a pan could strand the page off screen.
    private func clamp(_ candidate: CGPoint, atScale: CGFloat,
                       viewport: CGSize, baseW: CGFloat, baseH: CGFloat) -> CGPoint {
        let contentW = baseW * atScale
        let contentH = baseH * atScale
        let x = contentW <= viewport.width
            ? (viewport.width - contentW) / 2
            : min(max(candidate.x, viewport.width - contentW), 0)
        let y = contentH <= viewport.height
            ? (viewport.height - contentH) / 2
            : min(max(candidate.y, viewport.height - contentH), 0)
        return CGPoint(x: x, y: y)
    }

    /// Scale about `focus`, a point in viewport coordinates.
    ///
    /// The content point under the focus is `(focus − offset) / scale`. For it to
    /// stay under the focus at the new scale the offset must become
    /// `focus − thatPoint × newScale`, which reduces to the expression below.
    private func zoomAbout(_ factor: CGFloat, focus: CGPoint,
                           viewport: CGSize, baseW: CGFloat, baseH: CGFloat) {
        let newScale = Zoom.clamp(scale * factor)
        guard newScale != scale else {
            // Recorded rather than swallowed: a pinch that changes nothing is
            // either at a limit or being fed a factor of one, and those are very
            // different bugs.
            SessionRecorder.shared.record("ZOOM_TOUCH",
               String(format: "pinned refused factor=%.3f scale=%.2f", factor, scale))
            return
        }
        let ratio = newScale / scale
        offset = clamp(CGPoint(x: focus.x - (focus.x - offset.x) * ratio,
                               y: focus.y - (focus.y - offset.y) * ratio),
                       atScale: newScale, viewport: viewport, baseW: baseW, baseH: baseH)
        scale = newScale
        SessionRecorder.shared.record("ZOOM_TOUCH",
           String(format: "pinned scale=%.2f at=%.0f,%.0f off=%.0f,%.0f",
                  newScale, focus.x, focus.y, offset.x, offset.y))
    }

    /// Let the pull go: turn the page if it went far enough, or spring back.
    ///
    /// Nowhere to go — the first page or the last — springs back too, which is
    /// what says the *document* has ended rather than the page.
    private func settle() {
        let pulled = pull
        let towards: Int = pulled <= -Self.pullToTurn ? 1 : (pulled >= Self.pullToTurn ? -1 : 0)
        SessionRecorder.shared.record("DRAG_SCROLL", String(format: "pull=%.0f towards=%d", pulled, towards))
        if towards != 0, onTurnPage(towards) {
            landing = towards
            pull = 0
            return
        }
        guard pulled != 0 else { return }
        withAnimation(.easeOut(duration: 0.2)) { pull = 0 }
    }

    // ----------------------------------------------------------- gestures --

    private func pan(viewport: CGSize, baseW: CGFloat, baseH: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 1)
            .onChanged { value in
                // The reader is moving under their own hand now, so wherever the
                // turn put them is where they are.
                landing = nil
                let wanted = CGPoint(x: offset.x + value.translation.width - lastDrag.width,
                                     y: offset.y + value.translation.height - lastDrag.height)
                let held = clamp(wanted, atScale: scale,
                                 viewport: viewport, baseW: baseW, baseH: baseH)
                offset = held
                lastDrag = value.translation

                // Whatever the clamp refused vertically is the page having run
                // out. It gives, dampened, up to a limit.
                let refused = wanted.y - held.y
                if refused != 0 {
                    pull = min(max(pull + refused * Self.pullDamping, -Self.pullLimit),
                               Self.pullLimit)
                }
            }
            .onEnded { _ in
                lastDrag = .zero
                settle()
            }
    }

    @State private var lastDrag: CGSize = .zero

    // ---------------------------------------------------------- rendering --

    /// Draw the page again.
    ///
    /// - Parameter force: redraw even though the zoom has not increased. The scale
    ///   guard below exists so that pinching *out* does not throw away a sharper
    ///   raster for a blurrier one — but a caption that moved changes the page's
    ///   content at exactly the same scale, so that guard silently skipped the
    ///   redraw and left the old words baked into the picture while the overlay
    ///   drew them in their new place. Two copies, one of them a ghost.
    private func rasterise(baseW: CGFloat, force: Bool = false) async {
        guard let size = pageSize ?? (try? document.pageSize(pageIndex)) else { return }
        let wanted = RenderScale.forPage(size,
                                         targetPixelWidth: baseW * displayScale * committedScale)
        // Never coarser than what is already on screen.
        let target = force ? max(wanted, imageScale) : wanted
        guard force || target > imageScale else { return }

        let document = document
        let index = pageIndex
        let began = DispatchTime.now().uptimeNanoseconds
        let drawn = await Task.detached(priority: .userInitiated) {
            try? document.render(page: index, scale: target)
        }.value

        // A superseded render is thrown away rather than drawn.
        //
        // `.task(id:)` cancels the previous task when the id changes, but the
        // detached render inside it carries on to completion — so a run of
        // invalidations put three full-page renders in flight at once, each
        // finishing and assigning over the last. The recording showed exactly
        // that: three `PAGE_READABLE` lines on the same millisecond.
        guard !Task.isCancelled else { return }

        // Assigned only on success, so a failed or slow render leaves whatever is
        // on screen in place rather than clearing it.
        if let drawn {
            SessionRecorder.shared.record("PAGE_READABLE",
               String(format: "pinned page=%d px=%dx%d scale=%.2f",
                      index, drawn.width, drawn.height, target),
               durationMillis: Int((DispatchTime.now().uptimeNanoseconds &- began) / 1_000_000))
            image = drawn
            imageScale = target
        }
        didDraw = true
    }
}
