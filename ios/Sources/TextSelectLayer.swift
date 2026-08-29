import SwiftUI
import UIKit

/// The touches that select text on a page.
///
/// UIKit rather than a SwiftUI gesture, for the reason everything else that has
/// to be reliable here is UIKit: a `Canvas` has no backing view, so a recogniser
/// attached to one is attached to nothing.
///
/// Invisible to hit testing, and its recognisers fitted to the **hosting view** —
/// the same trick, and for the same reasons, as `GestureHostView` in
/// PinchToZoom.swift, whose comment is worth reading before changing anything
/// here. This layer began life hit-testable, which is the obvious way to make a
/// recogniser hear a finger and the wrong one: a plain `UIView` filling the page
/// is *above* the magnified page's own gestures, and a sibling above them takes
/// every touch they were waiting for. Arming the selection layer therefore
/// switched off one-finger panning and double-tap zoom in the magnified view —
/// both of which are attached to the base beneath this overlay — and did it
/// silently, because a starved gesture looks exactly like a missing one.
///
/// The two prices of reaching up to the hosting view are paid below: touches
/// meant for the toolbars are refused by bounds, and every position is reported
/// in this view's own space.
struct TextSelectLayer: UIViewRepresentable {
    /// A long press landed, in view coordinates.
    let onLongPress: (CGPoint) -> Void
    /// A plain tap landed on the page, whether or not words are selected.
    ///
    /// Separate from `onTapAway`, which only fires while a selection stands: a
    /// tap has to be able to *take* a drawn mark in hand as well as put one down.
    var onTap: (CGPoint) -> Void = { _ in }
    /// Where the drawn mark in hand is on screen, if one is. A drag that starts
    /// inside it carries it; anywhere else the finger belongs to the scroller.
    var heldMark: CGRect?
    /// The mark was carried this far, and whether the finger has lifted.
    var onMoveMark: (CGSize, Bool) -> Void = { _, _ in }
    /// Where the two handles are, in view coordinates. Nil when there is no
    /// selection, which is what turns handle dragging and tap-to-clear off.
    var handles: (start: CGPoint, end: CGPoint)?
    /// A handle is being dragged: which one, and where it is now.
    var onMoveHandle: (Bool, CGPoint) -> Void = { _, _ in }
    /// A plain tap landed while something was selected.
    var onTapAway: () -> Void = {}
    /// Something is selected — anywhere in the reader, not only on this page.
    ///
    /// The drag is armed per page, because only one page owns the handles. The
    /// tap is armed on **every** page, which is Android's asymmetry
    /// (`detectTapGestures` sits in the unconditional branch there) and is the
    /// whole escape route: with the list's scrolling suppressed while words are
    /// selected, a tap that only worked on the owning page would leave a reader
    /// on any other page with nothing that puts the selection down.
    var selectionActive: Bool = false

    func makeUIView(context: Context) -> UIView {
        let view = SelectTouchView()
        view.backgroundColor = .clear

        let press = UILongPressGestureRecognizer(target: view, action: #selector(SelectTouchView.press(_:)))
        press.minimumPressDuration = 0.5
        press.allowableMovement = 12

        let drag = UIPanGestureRecognizer(target: view, action: #selector(SelectTouchView.drag(_:)))
        let tap = UITapGestureRecognizer(target: view, action: #selector(SelectTouchView.tapped(_:)))

        for recogniser in [press, drag, tap] as [UIGestureRecognizer] {
            recogniser.cancelsTouchesInView = false
            recogniser.delaysTouchesBegan = false
            recogniser.delaysTouchesEnded = false
            recogniser.delegate = view
        }
        let markDrag = UIPanGestureRecognizer(target: view,
                                              action: #selector(SelectTouchView.carry(_:)))
        markDrag.cancelsTouchesInView = false
        markDrag.delaysTouchesBegan = false
        markDrag.delegate = view
        view.markDrag = markDrag

        view.handleDrag = drag
        view.tapAway = tap
        view.recognisers = [press, drag, tap, markDrag]
        view.apply(self)
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        (view as? SelectTouchView)?.apply(self)
    }

    static func dismantleUIView(_ view: UIView, coordinator: ()) {
        (view as? SelectTouchView)?.detach()
    }
}

private final class SelectTouchView: UIView, UIGestureRecognizerDelegate {
    private var onLongPress: (CGPoint) -> Void = { _ in }
    private var onMoveHandle: (Bool, CGPoint) -> Void = { _, _ in }
    private var onTapAway: () -> Void = {}
    private var handles: (start: CGPoint, end: CGPoint)?
    private var selectionActive = false
    private var onTap: (CGPoint) -> Void = { _ in }
    private var heldMark: CGRect?
    private var onMoveMark: (CGSize, Bool) -> Void = { _, _ in }
    weak var markDrag: UIPanGestureRecognizer?
    /// Which handle the drag took hold of, decided when it began.
    private var dragging: Bool?

    var recognisers: [UIGestureRecognizer] = []
    weak var handleDrag: UIPanGestureRecognizer?
    weak var tapAway: UITapGestureRecognizer?

    func apply(_ layer: TextSelectLayer) {
        onLongPress = layer.onLongPress
        onMoveHandle = layer.onMoveHandle
        onTapAway = layer.onTapAway
        handles = layer.handles
        selectionActive = layer.selectionActive
        onTap = layer.onTap
        heldMark = layer.heldMark
        onMoveMark = layer.onMoveMark
        // Armed only while a mark is in hand, and even then it begins only inside
        // that mark — see `gestureRecognizerShouldBegin`. A pan armed over the
        // whole page would take the scroller's finger everywhere a mark exists.
        markDrag?.isEnabled = layer.heldMark != nil
        // The drag only where the handles are; the tap wherever a selection
        // stands. Recognisers armed with nothing to do only give the scroll view
        // something to argue with.
        handleDrag?.isEnabled = layer.handles != nil
        // Always armed now: the tap both takes a drawn mark in hand and puts a
        // selection down, and only the second of those needs one to exist.
        tapAway?.isEnabled = true
    }

    // ------------------------------------------------------------- the host --

    /// Never a touch target. See the type's comment: a view that swallows touches
    /// starves the gestures layered beneath it.
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? { nil }

    /// The view every touch on this screen passes through.
    ///
    /// UIKit builds a touch's recogniser set once, at `touchesBegan`, from the
    /// hit-test view and its ancestors. A finger on bare page hit-tests straight
    /// to the hosting view, so only a recogniser fitted that high is on every
    /// touch's chain.
    private var gestureHost: UIView? {
        var candidate: UIView = self
        while let next = candidate.superview, !(next is UIWindow) { candidate = next }
        return candidate === self ? nil : candidate
    }

    func detach() {
        for recogniser in recognisers { recogniser.view?.removeGestureRecognizer(recogniser) }
    }

    deinit { detach() }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return detach() }
        guard let host = gestureHost else { return }
        for recogniser in recognisers where recogniser.view !== host {
            // Exactly one of each survives whatever churn SwiftUI puts this view
            // through: the host outlives it, so a recogniser left behind would
            // stack up and, with its delegate released, answer "no" to running
            // alongside anything — starving the live ones.
            recogniser.view?.removeGestureRecognizer(recogniser)
            host.addGestureRecognizer(recogniser)
        }
    }

    // -------------------------------------------------------------- touches --

    @objc func tapped(_ recogniser: UITapGestureRecognizer) {
        // Both, in this order: a tap that lands on a drawn mark takes it in hand,
        // and one that lands on bare page puts down whatever was — words or mark.
        onTap(recogniser.location(in: self))
        if selectionActive { onTapAway() }
    }

    @objc func carry(_ recogniser: UIPanGestureRecognizer) {
        let offset = recogniser.translation(in: self)
        let travelled = CGSize(width: offset.x, height: offset.y)
        switch recogniser.state {
        case .changed: onMoveMark(travelled, false)
        case .ended:   onMoveMark(travelled, true)
        case .cancelled, .failed: onMoveMark(.zero, true)
        default: break
        }
    }

    @objc func press(_ recogniser: UILongPressGestureRecognizer) {
        guard recogniser.state == .began else { return }
        onLongPress(recogniser.location(in: self))
    }

    @objc func drag(_ recogniser: UIPanGestureRecognizer) {
        switch recogniser.state {
        case .began:
            dragging = nearestHandle(to: recogniser.location(in: self))
        case .changed:
            guard let isStart = dragging else { return }
            onMoveHandle(isStart, recogniser.location(in: self))
        default:
            dragging = nil
        }
    }

    /// Which end of the selection a drag is moving: whichever it started nearer.
    ///
    /// Any drag, from anywhere on the page — not only one that began on a handle.
    /// That is Android's rule (`detectDragGestures` inside
    /// `if (selection != null)`, with no proximity test, and `change.consume()`
    /// so the scroller never sees it), and it is the better one: once words are
    /// selected the next drag is always about the selection, and a handle is a
    /// small thing to have to hit before the gesture means anything.
    ///
    /// Grabbing the wrong end collapses the selection, which reads as the drag
    /// having deleted it — hence *nearer*, measured once when the finger lands.
    private func nearestHandle(to point: CGPoint) -> Bool? {
        guard let handles else { return nil }
        let toStart = hypot(point.x - handles.start.x, point.y - handles.start.y)
        let toEnd = hypot(point.x - handles.end.x, point.y - handles.end.y)
        return toStart <= toEnd
    }

    /// Only touches that landed on this page.
    ///
    /// The recognisers sit on the hosting view, which hears the toolbars, the
    /// rail and every other page as well. Without this a long press on the tool
    /// ribbon would select a word behind it.
    ///
    /// The selection bar floats over the pages, so a tap on **Copy** is inside a
    /// page's bounds and does fire the tap-away as well. That is left alone on
    /// purpose: both buttons end by putting the selection down anyway, so the
    /// clear is the right end state either way. What was actually broken is that
    /// they *read* the live selection, and so read the one the tap had just
    /// cleared — fixed by handing them the selection the bar was built for. An
    /// attempt to exclude the bar by hit-test view was tried and reverted: in the
    /// list a finger on bare page hit-tests to the scroll view, not the hosting
    /// view, so the test took the long press with it.
    func gestureRecognizer(_ recogniser: UIGestureRecognizer,
                           shouldReceive touch: UITouch) -> Bool {
        bounds.contains(touch.location(in: self))
    }

    /// While words are selected, a drag on this page belongs to the selection.
    ///
    /// The recogniser is only enabled on the page that owns the selection, and
    /// `shouldReceive` keeps it to that page's bounds — so the rail, the toolbars
    /// and every other page scroll as they always did. Clearing the selection
    /// disarms it and the page scrolls again, which is the escape route: a tap
    /// still puts the selection down, because the tap recogniser runs alongside
    /// everything and a tap never satisfies a pan.
    override func gestureRecognizerShouldBegin(_ recogniser: UIGestureRecognizer) -> Bool {
        if recogniser === markDrag {
            guard let held = heldMark else { return false }
            return held.insetBy(dx: -22, dy: -22)
                .contains(recogniser.location(in: self))
        }
        guard recogniser === handleDrag else { return true }
        return handles != nil
    }

    func gestureRecognizer(_ recogniser: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        // The press and the tap run alongside anything — a scroll that outruns the
        // press's slop fails it on its own, which is the arbitration wanted.
        // Neither drag does: once a handle or a mark is in hand the page holds
        // still under it.
        recogniser !== handleDrag && recogniser !== markDrag
    }
}
