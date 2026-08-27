import Foundation
import SwiftUI

/// Dragging pages around a grid to reorder them.
///
/// Replaces a pair of nudge arrows on each cell. Two things were wrong with
/// those, and this fixes both. Moving a page five places meant five taps — and
/// each tap was a *document edit*, so each one re-rendered every thumbnail in
/// the grid. The sheet spent most of a reorder redrawing.
///
/// A drag moves the page in a list held here, on screen only. Nothing reaches
/// the document until the finger lifts, and then it is one move: one edit, one
/// re-render, one entry in the undo history to reverse it with.
///
/// ## Why the position is absolute
///
/// The obvious way to draw a dragged cell is to hand `DragGesture`'s
/// `translation` straight to `.offset`. It does not work here, and the way it
/// fails is exactly what it looks like: **the page jumps a whole cell sideways
/// the moment it passes a neighbour.**
///
/// The reason is that the page is also moving in the *list*. When the order
/// changes, the grid lays that cell out at its new slot, and `.offset` displaces
/// it from wherever the grid put it — so the thing the translation is added to
/// has itself jumped a cell, and the drawn result jumps with it.
///
/// So nothing here is relative to a slot. The cell's absolute position at the
/// start of the drag is remembered, and where it should be drawn now is
///
///     startPosition + howFarTheFingerHasMoved
///
/// which does not depend on slots at all. `displacement(slot:)` turns that into
/// an offset by subtracting wherever the grid has *currently* laid the cell out.
/// Whatever the layout does, the two cancel and the page stays under the finger.
/// It settles into place when the finger lifts, and not before.
///
/// The grid's own geometry comes back through preferences rather than being
/// asked for: SwiftUI has no `LazyGridState` to interrogate, so every cell
/// reports the frame it was given and this collects them.
final class GridReorderState: ObservableObject {
    /// Called once, on drop, with where the page started and where it ended.
    ///
    /// A property rather than an initialiser argument because this object
    /// outlives any single `body`, while the model it has to call is handed to
    /// the view afresh each time one runs.
    var onMove: (Int, Int) -> Void = { _, _ in }

    /// Which slot the dragged page currently occupies, or nil when nothing is
    /// being dragged. Its neighbours have made room.
    @Published private(set) var slot: Int?

    /// The order the grid should draw, as page indices.
    ///
    /// Empty when nothing is being dragged, which is most of the time.
    @Published private(set) var order: [Int] = []

    /// A scroll the drag has asked for. The grid acts on it and it stays put
    /// until the next one is different.
    @Published private(set) var scrollTo: GridScroll?

    /// The coordinate space every measurement here is in — the scroll view's
    /// own, so a frame moves when the grid scrolls, which is what
    /// `contentMoved(to:)` exists to answer for.
    ///
    /// Per instance rather than a shared name, so two grids on screen together
    /// cannot read each other's geometry.
    let space = UUID()

    /// Where each laid-out slot sits. Rebuilt by the grid on every layout pass,
    /// so slots that have scrolled out of the lazy stack simply stop appearing.
    ///
    /// **Not** `@Published`, and that is load-bearing. These frames come from a
    /// preference the grid's own cells publish, so announcing a change re-renders
    /// the grid, which re-measures the cells, which publishes again — sixty times
    /// a second, for ever. The grid then fights every scroll and rebuilds the
    /// views a drag is attached to, which is why neither scrolling nor reordering
    /// worked. Nothing in any `body` reads this; it is consulted only by the
    /// methods below, while a drag is in flight, and the things a body *does*
    /// watch — `slot`, `order`, `travelled` — are still published.
    private(set) var frames: [Int: CGRect] = [:]

    /// How far a finger may stray and still be a hold rather than a swipe.
    static let holdSlop: CGFloat = 12

    /// Where the page sat when the drag began.
    @Published private(set) var startPosition: CGPoint = .zero

    /// How far the finger has travelled since.
    @Published private(set) var travelled: CGSize = .zero

    /// Where the dragged page started, for the single move on drop.
    private var origin: Int?

    private var viewport: CGRect = .zero

    /// The top of the grid's content in the viewport, so a scroll can be
    /// measured rather than inferred.
    private var contentTop: CGFloat = 0

    /// How hard the drag is pressing into an edge, -1 (up) to 1 (down), 0 when
    /// it is nowhere near one.
    ///
    /// Written by the drag and read by the ticker, which is what lets one timer
    /// serve a speed that changes continuously.
    private var speed: CGFloat = 0

    /// The edge scroll, if one is running.
    private var ticker: Timer?

    /// Rows owed but not yet taken, so a fractional speed still adds up to a
    /// scroll rather than being rounded away every tick.
    private var owed: CGFloat = 0

    func isDragging(slot: Int) -> Bool { self.slot == slot }

    func start(count: Int, from slot: Int) {
        guard count > 0, let cell = frames[slot] else { return }
        order = Array(0..<count)
        origin = slot
        self.slot = slot
        startPosition = cell.origin
        travelled = .zero
    }

    /// Follow the finger, and shuffle the order when the page's own centre
    /// crosses into another cell.
    ///
    /// The page's centre rather than the finger: a page picked up by its corner
    /// would otherwise swap far too eagerly on one side and not at all on the
    /// other.
    func drag(to translation: CGSize) {
        guard let currentSlot = slot, let size = frames[currentSlot]?.size else { return }
        travelled = translation

        let centre = CGPoint(x: startPosition.x + translation.width + size.width / 2,
                             y: startPosition.y + translation.height + size.height / 2)

        scrollIfNearAnEdge(centre.y)

        // Only what is on screen. A finger dragged past the bottom of the sheet
        // has a centre that lands in rows the lazy stack has built but nobody
        // can see, and shuffling into one of those moves a page somewhere the
        // reader never pointed at.
        // Searched in slot order, the way Compose walks `visibleItemsInfo`. A
        // dictionary iterates in whatever order it likes, so the same drag could
        // resolve to a different cell on different runs.
        guard let target = frames.keys.sorted().first(where: { slot in
            guard slot != currentSlot, let frame = frames[slot] else { return false }
            // The viewport filter applies only when the viewport is known. It
            // arrived as zero — every candidate then failed `intersects`, so no
            // target was ever found and a dragged page could not change slot at
            // all. An unmeasured viewport must not silently disable reordering;
            // `frames` already holds only the cells the lazy grid has built.
            let onScreen = viewport.isEmpty || frame.intersects(viewport)
            return onScreen && frame.contains(centre)
        }), order.indices.contains(target) else { return }

        // Moved, not swapped. Swapping sends the displaced page all the way back
        // to where the dragged one came from, which across four pages is not
        // what anybody meant.
        order.insert(order.remove(at: currentSlot), at: target)
        slot = target
        // Deliberately no adjustment to `travelled`. The drawn position is
        // absolute, so the layout moving underneath changes nothing about where
        // this page appears — see the note on the type.
    }

    /// A drag that was cancelled rather than dropped.
    ///
    /// SwiftUI does not deliver `onEnded` when a gesture is cancelled or fails,
    /// so without this the autoscroll timer keeps firing after the finger has
    /// gone — scrolling a grid nobody is dragging in.
    func cancel() {
        clear()
    }

    func drop() {
        let from = origin
        let to = slot
        clear()
        if let from, let to, from != to { onMove(from, to) }
    }

    /// How far to displace the cell at `slot` from wherever the grid put it.
    ///
    /// Zero for every cell except the one being dragged.
    func displacement(slot: Int) -> CGSize {
        guard slot == self.slot, let laidOut = frames[slot] else { return .zero }
        return CGSize(width: startPosition.x + travelled.width - laidOut.minX,
                      height: startPosition.y + travelled.height - laidOut.minY)
    }

    // ------------------------------------------------------------ geometry --

    func slotsLaidOut(_ measured: [Int: CGRect]) {
        frames = measured
    }


    func viewportChanged(to size: CGSize) {
        viewport = CGRect(origin: .zero, size: size)
    }

    /// The grid scrolled, whoever scrolled it.
    ///
    /// A scroll moves every cell, the dragged one included — so the position it
    /// started from has to move with it, or the page slides out from under the
    /// finger by however far the grid travelled.
    func contentMoved(to top: CGFloat) {
        defer { contentTop = top }
        guard slot != nil else { return }
        startPosition.y += top - contentTop
    }

    // ---------------------------------------------------------- edge scroll --

    /// Scroll when the drag reaches the top or bottom of the grid.
    ///
    /// Without it a page can only be moved as far as the screen, and the last
    /// page of a long document could never be dragged to the front at all.
    ///
    /// One ticker, started when the drag enters an edge and stopped when it
    /// leaves. The Android build learned this the expensive way: a scroll
    /// launched per drag event meant dozens a second, all at once, and a page
    /// nudged towards the bottom shot to the end of the document before anybody
    /// could let go. The ticker reads `speed` afresh each time it fires, so a
    /// depth that changes continuously needs no restarting.
    private func scrollIfNearAnEdge(_ y: CGFloat) {
        let top = viewport.minY
        let bottom = viewport.maxY

        if y < top + Self.edge {
            speed = -depth(into: top + Self.edge - y)
        } else if y > bottom - Self.edge {
            speed = depth(into: y - (bottom - Self.edge))
        } else {
            speed = 0
        }

        SessionRecorder.shared.record("TOOL_GESTURE",
            String(format: "edge y=%.0f view=%.0f..%.0f speed=%.2f", y, top, bottom, speed))
        if speed == 0 {
            stopTicking()
        } else {
            startTicking()
        }
    }

    /// How far into the edge zone, from 0 at its outer limit to 1 at the very
    /// edge, so the scroll eases in rather than snapping to full pelt.
    private func depth(into distance: CGFloat) -> CGFloat {
        min(max(distance / Self.edge, 0), 1)
    }

    private func startTicking() {
        guard ticker == nil else { return }
        let timer = Timer(timeInterval: Self.tick, repeats: true) { [weak self] _ in
            self?.tick()
        }
        // `.common`, because a drag puts the run loop in tracking mode and a
        // timer in the default mode alone stops firing for exactly as long as
        // the finger is down — which is the whole time this is wanted.
        RunLoop.main.add(timer, forMode: .common)
        ticker = timer
    }

    private func stopTicking() {
        ticker?.invalidate()
        ticker = nil
        owed = 0
    }

    private func tick() {
        guard speed != 0 else { return stopTicking() }
        owed += abs(speed) * Self.rowsPerSecond * CGFloat(Self.tick)
        guard owed >= 1 else { return }
        owed -= 1
        if !scrollOneRow() { stopTicking() }
    }

    /// Ask the grid for one more row in the direction being pressed.
    ///
    /// False when there is no row left to ask for, which is how the ticker
    /// learns it has reached an end of the document.
    private func scrollOneRow() -> Bool {
        guard !order.isEmpty else { return false }
        let onScreen = frames.filter { $0.value.intersects(viewport) }
        guard let first = onScreen.keys.min(), let last = onScreen.keys.max() else { return false }

        let row = max(1, columnsOnScreen(onScreen))
        let wanted = speed < 0
            ? GridScroll(slot: max(0, first - row), anchor: .top)
            : GridScroll(slot: min(order.count - 1, last + row), anchor: .bottom)

        guard wanted != scrollTo else { return false }
        scrollTo = wanted
        return true
    }

    /// How many cells the grid fits across, read off the top row it is showing.
    ///
    /// Asked of the layout rather than passed in, because the column count is
    /// `GridItem(.adaptive:)`'s answer and nobody else's — it changes with the
    /// sheet's width and with the device being turned.
    private func columnsOnScreen(_ onScreen: [Int: CGRect]) -> Int {
        guard let top = onScreen.values.map(\.minY).min() else { return 1 }
        return onScreen.values.filter { abs($0.minY - top) < 1 }.count
    }

    private func clear() {
        stopTicking()
        speed = 0
        origin = nil
        slot = nil
        travelled = .zero
        startPosition = .zero
        order = []
        scrollTo = nil
    }

    /// How long a page has to be held before it lifts.
    ///
    /// After a hold, not immediately: the grid scrolls, and a cell that moved
    /// the moment a finger touched it would make scrolling impossible.
    static let hold: TimeInterval = 0.28

    /// How much a page grows while it is held, so it reads as picked up.
    static let lifted: CGFloat = 1.06

    /// How close to an edge starts a scroll, in points. Android's is 140
    /// pixels, which is about this on a three-times screen.
    /// How close to an edge a dragged page must come before the grid follows it.
    ///
    /// Was 48, which in a grid two rows tall is reached simply by moving a page
    /// from the first row to the second — so the window crept down on a drag that
    /// was nowhere near the end. Half that is still a comfortable target for
    /// deliberately dragging a page past the bottom, and stops the grid moving
    /// under an ordinary reorder.
    private static let edge: CGFloat = 24

    /// How fast the grid scrolls at the very edge.
    ///
    /// Quick enough to cross a long document without being quick enough to lose
    /// track of where the page is going.
    private static let rowsPerSecond: CGFloat = 1.5

    private static let tick: TimeInterval = 1.0 / 60
}

/// A scroll the drag has asked the grid for: bring `slot` into view against
/// `anchor`.
///
/// Equatable so the grid can tell a fresh request from the one it has already
/// carried out — and so a request that stops changing is how the ticker finds
/// out it has run out of document.
struct GridScroll: Equatable {
    let slot: Int
    let anchor: UnitPoint
}

extension View {
    /// Wraps the `ScrollView` a reorderable grid lives in.
    ///
    /// Everything the drag needs to know about the layout arrives here: the
    /// coordinate space the measurements are in, the viewport it compares them
    /// against, and the scroll position it has to compensate for.
    func reorderableGrid(_ state: GridReorderState) -> some View {
        modifier(ReorderableGrid(state: state))
    }

    /// Goes on the grid *inside* that scroll view, and reports where its top
    /// has got to.
    func reorderableContent(_ state: GridReorderState) -> some View {
        background {
            GeometryReader { geometry in
                Color.clear.preference(key: GridContentTopKey.self,
                                       value: geometry.frame(in: .named(state.space)).minY)
            }
        }
    }

    /// Makes one cell draggable after a hold.
    ///
    /// `slot` is the position in the grid, not the page — and the cell it is
    /// attached to has to be identified by the slot too. Identifying cells by
    /// page is what broke this on Android: a drag *changes* which slot holds a
    /// page, so the gesture handler was torn down and rebuilt the instant the
    /// page passed a neighbour, arriving as a cancel. The gesture could never
    /// survive its own success.
    func reorderable(_ state: GridReorderState,
                     slot: Int,
                     count: Int,
                     enabled: Bool) -> some View {
        modifier(Reorderable(state: state, slot: slot, count: count, enabled: enabled))
    }

    /// Goes on the **thumbnail** inside that cell, and nothing else.
    ///
    /// The buttons beside it are outside this on purpose: on Android they are
    /// siblings of the reorder modifier, so a hold on rotate or delete can never
    /// lift the page.
    func reorderHandle(_ state: GridReorderState,
                       slot: Int,
                       count: Int,
                       enabled: Bool,
                       onTap: @escaping () -> Void = {}) -> some View {
        modifier(ReorderHandle(state: state, slot: slot, count: count,
                               enabled: enabled, onTap: onTap))
    }
}

private struct ReorderableGrid: ViewModifier {
    @ObservedObject var state: GridReorderState

    func body(content: Content) -> some View {
        ScrollViewReader { proxy in
            content
                .coordinateSpace(.named(state.space))
                .background {
                    GeometryReader { geometry in
                        Color.clear.preference(key: GridViewportKey.self, value: geometry.size)
                    }
                }
                .onPreferenceChange(GridSlotFramesKey.self) { state.slotsLaidOut($0) }
                .onPreferenceChange(GridViewportKey.self) { state.viewportChanged(to: $0) }
                .onPreferenceChange(GridContentTopKey.self) { state.contentMoved(to: $0) }
                .onChange(of: state.scrollTo) { _, wanted in
                    guard let wanted else { return }
                    SessionRecorder.shared.record("TOOL_GESTURE",
                        "grid scrollTo slot=\(wanted.slot)")
                    proxy.scrollTo(wanted.slot, anchor: wanted.anchor)
                }
        }
    }
}

private struct Reorderable: ViewModifier {
    /// True only while the gesture is live. SwiftUI resets a `@GestureState`
    /// on cancellation, which is the only notice a cancelled drag gives.
    @GestureState private var holding = false
    @ObservedObject var state: GridReorderState
    let slot: Int
    let count: Int
    let enabled: Bool

    private var dragging: Bool { state.isDragging(slot: slot) }

    func body(content: Content) -> some View {
        // The stack is what holds the grid slot, and only its contents move.
        // Measuring the slot from inside the offset view would measure the
        // displacement as well, and the page would chase its own tail across
        // the sheet.
        ZStack {
            content
                .scaleEffect(dragging ? GridReorderState.lifted : 1)
                .offset(state.displacement(slot: slot))
        }
        .background {
            GeometryReader { geometry in
                Color.clear.preference(key: GridSlotFramesKey.self,
                                       value: [slot: geometry.frame(in: .named(state.space))])
            }
        }
        // Lifted out of the flow and drawn last, so the page being dragged
        // passes over its neighbours rather than under them.
        .zIndex(dragging ? 1 : 0)
        // Ahead of the cell's own tap, so a hold that becomes a drag is taken
        // here and never arrives there as a selection. A quick tap never
        // satisfies the hold, so it falls through untouched.
        //
        // `.subviews` rather than dropping the gesture, so a read-only document
        // still answers a tap on a page: the cell's own gestures survive, and
        // only the drag goes away.
    }
}

/// The half of the reorder that takes the finger.
///
/// Kept apart from the layout half on purpose. Android puts the drag on the
/// thumbnail alone — the rotate and delete buttons are siblings outside it — so a
/// hold on a button can never become a drag. Attaching it to the whole cell
/// instead makes holding either button lift the page.
struct ReorderHandle: ViewModifier {
    /// True only while the gesture is live. SwiftUI resets a `@GestureState` on
    /// cancellation, which is the only notice a cancelled drag gives.
    @GestureState private var holding = false
    @ObservedObject var state: GridReorderState
    let slot: Int
    let count: Int
    let enabled: Bool
    /// Choosing this page. It decides where a blank or an import lands, since
    /// both go after the page in hand.
    var onTap: () -> Void = {}

    func body(content: Content) -> some View {
        content.overlay {
            ReorderTouchLayer(state: state, slot: slot, count: count,
                              enabled: enabled, onTap: onTap)
        }
    }
}

/// One `UILongPressGestureRecognizer`, which is the whole gesture.
///
/// SwiftUI cannot express this. A `DragGesture` on a cell claims the pan of the
/// scroll view the grid lives in — as `.gesture`, as `.highPriorityGesture` and
/// as `.simultaneousGesture`, all three tried, all three leaving the grid
/// unscrollable; switching the gesture off made it scroll, which is what proved
/// the cause. Arming a drag only after a long press does not work either: a
/// gesture masked in while a touch is already in flight never adopts it, so the
/// page lifted and then refused to follow the finger.
///
/// A long-press recogniser has no such problem, and is what UIKit's own
/// collection views reorder with. It reports **both** phases — `.began` after the
/// hold, then `.changed` for every movement of the same unbroken touch — so there
/// is no second gesture to hand over to. It yields to the scroll view for free: a
/// finger that moves before the hold elapses fails it, and the scroller, which was
/// never blocked, simply carries on.
private struct ReorderTouchLayer: UIViewRepresentable {
    @ObservedObject var state: GridReorderState
    let slot: Int
    let count: Int
    let enabled: Bool
    var onTap: () -> Void = {}

    func makeUIView(context: Context) -> UIView {
        // Hit-testable, and it has to be.
        //
        // Making it transparent to touches and moving the hold onto the cell
        // beneath looked like the way to stop it swallowing taps — and killed
        // reordering outright, because SwiftUI draws a thumbnail with no backing
        // view of its own. Nothing was left in that subtree for a touch to
        // resolve to, so the recogniser was handed nothing at all. The tap is
        // answered here instead, which costs one more recogniser and no guessing.
        let view = UIView()
        view.backgroundColor = .clear
        let hold = UILongPressGestureRecognizer(target: context.coordinator,
                                                action: #selector(Coordinator.handle(_:)))
        hold.minimumPressDuration = GridReorderState.hold
        hold.allowableMovement = GridReorderState.holdSlop
        // The cell keeps its taps: a hold that never becomes one must still let
        // the page be selected, rotated or deleted.
        hold.cancelsTouchesInView = false
        hold.delegate = context.coordinator
        context.coordinator.hold = hold
        view.addGestureRecognizer(hold)

        // Held back until the hold has failed, so a page picked up is not also a
        // page chosen.
        let tap = UITapGestureRecognizer(target: context.coordinator,
                                         action: #selector(Coordinator.handleTap))
        tap.delegate = context.coordinator
        tap.require(toFail: hold)
        view.addGestureRecognizer(tap)
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        context.coordinator.state = state
        context.coordinator.slot = slot
        context.coordinator.count = count
        context.coordinator.onTap = onTap
        context.coordinator.hold?.isEnabled = enabled
    }

    func makeCoordinator() -> Coordinator {
        let coordinator = Coordinator(state: state, slot: slot, count: count)
        coordinator.onTap = onTap
        return coordinator
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var state: GridReorderState
        var slot: Int
        var count: Int
        weak var hold: UILongPressGestureRecognizer?
        var onTap: () -> Void = {}

        @objc func handleTap() { onTap() }
        private var origin: CGPoint = .zero
        /// The scroll view frozen for the duration of a drag, so it can be let go
        /// again afterwards.
        private weak var frozen: UIScrollView?
        /// The sheet this grid is in, pinned for the duration of a drag.
        private weak var pinned: UIViewController?
        /// The sheet's own pan gestures, switched off for the duration of a drag
        /// and switched back on at the drop.
        private var suspended: [UIPanGestureRecognizer] = []

        /// The view controller presenting this, found through the responder
        /// chain — a `UIView` does not name its own controller.
        private func controller(from view: UIView?) -> UIViewController? {
            var responder: UIResponder? = view
            while let next = responder {
                if let controller = next as? UIViewController { return controller }
                responder = next.next
            }
            return nil
        }

        private func scrollView(from view: UIView?) -> UIScrollView? {
            var candidate = view
            while let next = candidate {
                if let scroll = next as? UIScrollView { return scroll }
                candidate = next.superview
            }
            return nil
        }

        init(state: GridReorderState, slot: Int, count: Int) {
            self.state = state
            self.slot = slot
            self.count = count
            super.init()
        }

        /// Alongside anything else — **except** the pan of the scroll view.
        ///
        /// Saying yes to everything is what let the grid scroll away under a page
        /// being dragged. The two are only compatible before this recogniser has
        /// anything to say: a swipe never satisfies the hold, so the long press
        /// fails and the scroller wins it without ever consulting this. But once
        /// the hold *has* been recognised, the finger belongs to the page, and a
        /// scroller running alongside moves the grid out from under it.
        ///
        /// Refusing here is what actually stops it. Disabling the scroll view by
        /// hand is kept below as well, but it depends on finding that view, and
        /// this does not.
        func gestureRecognizer(_ recogniser: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
            !(other.view is UIScrollView)
        }

        /// Stop the sheet itself from moving.
        ///
        /// `isModalInPresentation` is not enough, and a recording settled why: a
        /// drag logged `speed=0.00` on all 87 of its frames and asked the grid to
        /// scroll exactly **zero** times. The grid was not moving at all — the
        /// sheet was. Marking a sheet non-dismissible stops it being *dismissed*;
        /// it does not stop UIKit rubber-banding it under a downward finger and
        /// springing it back, which is precisely the small dip that kept showing
        /// up. That pan belongs to the presentation, above this grid, so nothing
        /// done to the grid's own scroller could ever have reached it.
        ///
        /// It lives somewhere between the sheet's view and the window, so the
        /// chain is walked rather than guessed at. The grid's own scroller is
        /// underneath that view and is never met on the way up; a pan attached to
        /// a scroll view is skipped regardless.
        private func suspendSheetPan(from view: UIView?) {
            var candidate = controller(from: view)?.view
            while let next = candidate, !(next is UIWindow) {
                for gesture in next.gestureRecognizers ?? [] {
                    guard let pan = gesture as? UIPanGestureRecognizer,
                          pan.isEnabled, !(pan.view is UIScrollView) else { continue }
                    // Disabling mid-gesture cancels it, which is the point: the
                    // sheet lets go of the finger and stops following it.
                    pan.isEnabled = false
                    suspended.append(pan)
                }
                candidate = next.superview
            }
            SessionRecorder.shared.record("TOOL_GESTURE",
                "sheet pans suspended=\(suspended.count)")
        }

        /// Give the scroller, and the sheet, back.
        private func release() {
            frozen?.isScrollEnabled = true
            frozen = nil
            pinned?.isModalInPresentation = false
            pinned = nil
            suspended.forEach { $0.isEnabled = true }
            suspended.removeAll()
        }

        @objc func handle(_ recogniser: UILongPressGestureRecognizer) {
            guard let view = recogniser.view else { return }
            // Measured against the **window**, not this view.
            //
            // This view is inside the cell, and the cell moves with the drag — so
            // a finger travelling 195pt across the grid only travelled 80pt
            // relative to the thing it was dragging, and the page crawled along at
            // a fraction of the hand. The window holds still.
            let at = recogniser.location(in: view.window)
            switch recogniser.state {
            case .began:
                origin = at
                // Taken out of the gesture for as long as a page is lifted.
                //
                // The hold deliberately runs alongside the scroller — that is what
                // lets a swipe scroll instead of lifting a page — but once a page
                // is in hand the two stop being compatible, and dragging one up or
                // down scrolled the grid away underneath it. A SwiftUI
                // `.scrollDisabled` did not take hold mid-gesture; the scroll view
                // itself does.
                frozen = scrollView(from: recogniser.view)
                frozen?.isScrollEnabled = false
                // The viewport, taken from the scroll view itself.
                //
                // It was supposed to arrive as a preference and arrived as zero,
                // which made `scrollIfNearAnEdge` read `bottom = 0` and answer
                // "past the bottom edge" for **every** position on the sheet. It
                // therefore set a downward speed and kept it there for the whole
                // drag: the grid scrolled away under the page for as long as it
                // was held. That is programmatic scrolling, so neither disabling
                // the scroll view nor refusing its pan could touch it — three
                // fixes aimed at the wrong mechanism. Asking the view that owns
                // the viewport is the reliable answer.
                if let scroll = frozen {
                    state.viewportChanged(to: scroll.bounds.size)
                }

                // Pinned here, in UIKit, and not through SwiftUI.
                //
                // `.interactiveDismissDisabled` is bound to state that only flips
                // once the hold succeeds, and SwiftUI needs an update cycle to
                // carry that to the presentation controller. The finger is already
                // moving by then, so the sheet followed it down for a frame or two
                // before the brake arrived — a little movement, every time. This
                // takes effect on the same runloop turn as the lift.
                pinned = controller(from: recogniser.view)
                pinned?.isModalInPresentation = true
                suspendSheetPan(from: recogniser.view)
                state.start(count: count, from: slot)
            case .changed:
                // A translation, so no coordinate space has to agree with any
                // other: it is the same delta whichever view measures it.
                state.drag(to: CGSize(width: at.x - origin.x, height: at.y - origin.y))
            case .ended:
                release()
                state.drop()
            case .cancelled, .failed:
                release()
                state.cancel()
            default:
                break
            }
        }
    }
}

/// Where the grid has laid out each slot, collected from the cells themselves.
private struct GridSlotFramesKey: PreferenceKey {
    static var defaultValue: [Int: CGRect] { [:] }

    static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
        value.merge(nextValue()) { _, newer in newer }
    }
}

private struct GridViewportKey: PreferenceKey {
    static var defaultValue: CGSize { .zero }

    static func reduce(value: inout CGSize, nextValue: () -> CGSize) {
        value = nextValue()
    }
}

private struct GridContentTopKey: PreferenceKey {
    static var defaultValue: CGFloat { 0 }

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}
