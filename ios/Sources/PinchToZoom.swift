import SwiftUI
import UIKit

/// What a two-finger gesture turned out to be. Android's `core/TwoFingerGesture.kt`.
///
/// Two fingers dragging in parallel never stay exactly parallel: the distance
/// between them wanders by a few points a frame, and each wobble reaches the
/// pinch handler as a factor slightly off 1. Acting on them turns a two-finger
/// *scroll* into a slow zoom, so a gesture is classified before it is acted on
/// and the classification then holds until the fingers lift.
enum TwoFingerGesture {
    case undecided, zoom, pan

    /// How far either measurement must travel before the gesture is called.
    /// Comfortably above the noise of a parallel drag, well under a deliberate
    /// pinch.
    static let slop: CGFloat = 20

    /// Classify, or keep an existing decision.
    ///
    /// - Parameters:
    ///   - spreadChange: how far the fingers are from the separation they started
    ///     at. Pinching in and out are both zooming, so the magnitude is what
    ///     counts.
    ///   - centroidTravel: total distance the midpoint has travelled, summed
    ///     frame by frame rather than end to end, so a drag out and back still
    ///     counts as movement.
    static func classify(current: TwoFingerGesture,
                         spreadChange: CGFloat,
                         centroidTravel: CGFloat) -> TwoFingerGesture {
        // A decision, once made, is kept: a pan that drifts apart near its end
        // must not turn into a zoom under the reader's fingers.
        guard current == .undecided else { return current }

        // Zoom is tested first so a pinch anchored on one stationary finger still
        // reads as a zoom. That gesture moves the midpoint about half as far as it
        // changes the separation, so on the frame the separation passes the slop
        // the midpoint may have passed it too — and the pinch is what was meant.
        if abs(spreadChange) > slop { return .zoom }
        if centroidTravel > slop { return .pan }
        return .undecided
    }
}

/// Magnification reached so far in a pinch, before the magnified view exists.
///
/// A plain running product, deliberately **unclamped**. Clamping each step at 1
/// makes it a ratchet: the wobble of a two-finger scroll pushes it up and down in
/// equal measure, but only the upward half survives, so it creeps towards the
/// handover threshold and eventually zooms the reader in on its own — which is
/// why it would only ever go *in*. Any clamping belongs where the value is used.
func pinchProgressAfter(_ progress: CGFloat, _ factor: CGFloat) -> CGFloat {
    progress * factor
}

/// Two-finger zoom that leaves one-finger gestures alone. Android's
/// `ui/components/PinchToZoom.kt`.
///
/// The callback receives the scale change **and the centroid** — the midpoint
/// between the fingers, in this view's own coordinates. The caller needs it
/// because a pinch must keep the content under that midpoint stationary; scaling
/// about any fixed point instead — the corner, or where the gesture happened to
/// begin — makes the page slide out from under the fingers.
final class PinchRecogniser: UIGestureRecognizer {
    /// The scale change since the last emission, and where the fingers are now.
    var onZoomBy: ((CGFloat, CGPoint) -> Void)?
    /// Fired when the last finger lifts, so a caller accumulating across a
    /// gesture can reset — otherwise what one gesture left behind is still there
    /// when the next begins.
    var onGestureEnd: (() -> Void)?

    private var kind = TwoFingerGesture.undecided
    /// Finger separation when the second finger landed.
    private var startSpread: CGFloat = 0
    private var lastSpread: CGFloat = 0
    private var lastCentroid: CGPoint?
    private var centroidTravel: CGFloat = 0

    /// The overlay this gesture is *about*.
    ///
    /// The recogniser is fitted to the hosting view so it is handed every finger,
    /// but the caller wrote its maths against the overlay. Every point is reported
    /// through this, never through `view` — otherwise the centroid arrives in
    /// scene coordinates and the zoom anchors a top bar and a rail away from the
    /// fingers.
    weak var marker: UIView?

    private func measure(_ event: UIEvent?) -> (centroid: CGPoint, spread: CGFloat)? {
        guard let marker, numberOfTouches >= 2 else { return nil }

        let points = (0..<numberOfTouches).map { location(ofTouch: $0, in: marker) }
        let centroid = CGPoint(x: points.map(\.x).reduce(0, +) / CGFloat(points.count),
                               y: points.map(\.y).reduce(0, +) / CGFloat(points.count))
        // Mean distance from the centroid, the same measure Compose's
        // `calculateCentroidSize` uses.
        let spread = points.reduce(CGFloat(0)) {
            $0 + hypot($1.x - centroid.x, $1.y - centroid.y)
        } / CGFloat(points.count)
        return (centroid, spread)
    }

    private func update(_ event: UIEvent?) {
        guard let (centroid, spread) = measure(event) else { return }

        guard startSpread != 0 else {
            startSpread = spread
            lastSpread = spread
            lastCentroid = centroid
            return
        }

        if let previous = lastCentroid {
            centroidTravel += hypot(centroid.x - previous.x, centroid.y - previous.y)
        }
        lastCentroid = centroid

        kind = TwoFingerGesture.classify(current: kind,
                                         spreadChange: spread - startSpread,
                                         centroidTravel: centroidTravel)

        if kind == .zoom, lastSpread > 0 {
            let factor = spread / lastSpread
            if factor != 1, factor.isFinite, factor > 0 {
                state = state == .possible ? .began : .changed
                onZoomBy?(factor, centroid)
            }
        }
        lastSpread = spread
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        // Fitted to the hosting view, so it hears the whole screen. Anything that
        // did not land on the overlay is dropped here rather than being measured
        // — a finger on the toolbar is not part of this gesture.
        if let marker {
            for touch in touches where !marker.bounds.contains(touch.location(in: marker)) {
                ignore(touch, for: event)
            }
        }
        // Recorded on the way in, before any gate. A recording with no
        // `pinch touches=` line at all says the recogniser never sees a finger,
        // which is a completely different bug from one that sees fingers and
        // declines to act on them.
        SessionRecorder.shared.record("ZOOM_TOUCH", "pinch touches=\(numberOfTouches)")
        update(event)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        update(event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesEnded(touches, with: event)
        if numberOfTouches <= 1 {
            if startSpread != 0 { onGestureEnd?() }
            state = state == .possible ? .failed : .ended
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesCancelled(touches, with: event)
        if startSpread != 0 { onGestureEnd?() }
        state = .cancelled
    }

    override func reset() {
        super.reset()
        kind = .undecided
        startSpread = 0
        lastSpread = 0
        lastCentroid = nil
        centroidTravel = 0
    }
}

/// Fits the pinch over a view.
struct PinchToZoomLayer: UIViewRepresentable {
    let onZoomBy: (CGFloat, CGPoint) -> Void
    var onGestureEnd: () -> Void = {}

    func makeUIView(context: Context) -> UIView {
        let view = GestureHostView()
        let recogniser = PinchRecogniser(target: context.coordinator,
                                         action: #selector(Coordinator.noop))
        // Read through the coordinator, never captured: a closure built here
        // holds the first render's state for ever.
        recogniser.onZoomBy = { [weak coordinator = context.coordinator] factor, centroid in
            coordinator?.zoom(factor, centroid)
        }
        recogniser.onGestureEnd = { [weak coordinator = context.coordinator] in
            coordinator?.ended()
        }
        recogniser.delegate = context.coordinator
        // Parked here and moved onto the superview once this is in a window —
        // see `GestureHostView`.
        recogniser.marker = view
        view.recogniser = recogniser
        view.keepAlive = context.coordinator
        view.addGestureRecognizer(recogniser)
        return view
    }

    static func dismantleUIView(_ view: UIView, coordinator: Coordinator) {
        (view as? GestureHostView)?.detach()
    }

    func updateUIView(_ view: UIView, context: Context) {
        context.coordinator.onZoomBy = onZoomBy
        context.coordinator.onGestureEnd = onGestureEnd
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onZoomBy: onZoomBy, onGestureEnd: onGestureEnd)
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var onZoomBy: (CGFloat, CGPoint) -> Void
        var onGestureEnd: () -> Void

        init(onZoomBy: @escaping (CGFloat, CGPoint) -> Void,
             onGestureEnd: @escaping () -> Void) {
            self.onZoomBy = onZoomBy
            self.onGestureEnd = onGestureEnd
        }

        func zoom(_ factor: CGFloat, _ centroid: CGPoint) { onZoomBy(factor, centroid) }
        func ended() { onGestureEnd() }
        @objc func noop() {}

        /// Runs alongside the two-finger pan, which reads the same events.
        func gestureRecognizer(_ recogniser: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
            true
        }
    }
}

/// Hosts a gesture recogniser without taking part in hit testing itself.
///
/// This is the whole trick, and getting it wrong kills the gesture silently. A
/// view that returns nil from `hitTest` is never the target of a touch — and a
/// recogniser attached to *that* view therefore never receives one either. A view
/// that returns itself swallows every touch meant for the SwiftUI content beneath.
///
/// So the recogniser is moved onto the **superview**, which is the host that does
/// take part in hit testing, while this view stays invisible to touches. With
/// `cancelsTouchesInView` off, single fingers still reach everything underneath.
class GestureHostView: UIView {
    var recogniser: UIGestureRecognizer?

    /// The recogniser's delegate, held **strongly**.
    ///
    /// `UIGestureRecognizer.delegate` is a weak reference, and the delegate here
    /// is SwiftUI's coordinator — which SwiftUI releases when it tears the
    /// representable down. The recogniser, though, was moved onto an ancestor and
    /// outlives that: it carries on with a nil delegate, whose default answer to
    /// "may I run alongside another recogniser" is **no**. So a leaked recogniser
    /// does not merely linger, it actively starves the live pinch, the two-finger
    /// pan and the scroll view's own pan.
    var keepAlive: AnyObject?

    /// Take the recogniser off whatever it was fitted to.
    func detach() {
        guard let recogniser else { return }
        recogniser.view?.removeGestureRecognizer(recogniser)
    }

    deinit {
        // `addGestureRecognizer` retains, and the ancestor outlives this view, so
        // without this every rebuild left another live recogniser on the page.
        // A recording showed five pinch recognisers all reporting the same finger.
        recogniser?.view?.removeGestureRecognizer(recogniser!)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard let recogniser else { return }

        // Taken off again on the way out.
        //
        // The recogniser is fitted to an **ancestor**, which outlives this view.
        // Every time SwiftUI rebuilt the layer a fresh recogniser was added and
        // the old one left behind, so they stacked up on the page: a recording
        // showed five pinch recognisers all reporting the same finger, none of
        // them ever seeing the second one. Whatever churn puts this view through
        // the hierarchy, it leaves exactly one recogniser behind it.
        guard window != nil else {
            recogniser.view?.removeGestureRecognizer(recogniser)
            return
        }

        guard let host = gestureHost else { return }
        if recogniser.view !== host {
            recogniser.view?.removeGestureRecognizer(recogniser)
            recogniser.cancelsTouchesInView = false
            recogniser.delaysTouchesBegan = false
            recogniser.delaysTouchesEnded = false
            host.addGestureRecognizer(recogniser)
            SessionRecorder.shared.record("TOOL_GESTURE",
                "attached=\(type(of: recogniser)) to=\(String(describing: type(of: host)))")
        }
    }

    /// The view every touch on this screen passes through: the hosting view.
    ///
    /// UIKit builds a touch's recogniser set **once**, at `touchesBegan`, by
    /// hit-testing the touch and then walking from the hit-test view up the
    /// superview chain to the window, collecting recognisers on the way. A
    /// recogniser whose view is not the hit-test view or one of its ancestors is
    /// never handed that touch, and never revises the decision.
    ///
    /// SwiftUI builds backing UIViews only for representables. `Canvas`,
    /// `Color.clear`, `.contentShape` and the annotation overlays have none — so a
    /// finger on bare page hit-tests straight to the scene's hosting view, which
    /// is an **ancestor** of any overlay. Walking up from an ancestor never passes
    /// back down through the overlay, so that finger could not reach a recogniser
    /// fitted anywhere below it. The second finger of a pinch nearly always lands
    /// on bare page: a recording off the phone showed `touches=1` on every one of
    /// 125 events, which is that fact stated in numbers.
    ///
    /// Only the hosting view is guaranteed to be on every touch's chain. The two
    /// prices of reaching that high — hearing touches meant for the toolbars, and
    /// reporting in the wrong coordinate space — are paid explicitly below, by
    /// `marker`.
    private var gestureHost: UIView? {
        var candidate: UIView = self
        while let next = candidate.superview, !(next is UIWindow) {
            candidate = next
        }
        return candidate === self ? nil : candidate
    }

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? { nil }
}
