import SwiftUI
import UIKit

/// Scrolling with two fingers, for when one finger is busy annotating. Android's
/// `ui/components/TwoFingerPan.kt`.
///
/// While a tool is selected the list's own scrolling is switched off, because a
/// one-finger drag has to mean "draw" and cannot also mean "scroll" — the two
/// race, and the scroll container wins as soon as the drag passes touch slop, so
/// a highlight turns into a page scroll partway through.
///
/// This restores panning on a second finger. It reports only once two pointers
/// are down, so single-finger drags pass through untouched to the drawing layer.
final class TwoFingerPanRecogniser: UIGestureRecognizer {
    /// Centroid movement since the last event, sign matching the gesture:
    /// dragging down and to the right gives positive components.
    var onPan: ((CGSize) -> Void)?
    /// Two fingers went down, or came back up.
    ///
    /// The drawing layer needs this. Its `DragGesture` is handed the *first*
    /// finger and starts a stroke before the second one lands, and this
    /// recogniser runs alongside rather than instead of it — so a two-finger
    /// scroll with a pen armed left a line down the page it scrolled past.
    var onTwoFingers: ((Bool) -> Void)?

    private var lastCentroid: CGPoint?

    /// The overlay this gesture is about — see `PinchRecogniser.marker`. The
    /// recogniser is fitted to the hosting view so that both fingers reach it, and
    /// every point is reported back in the overlay's own space.
    weak var marker: UIView?

    /// The arithmetic mean of the touches this recogniser actually owns.
    private func centroid() -> CGPoint? {
        guard let marker, numberOfTouches >= 2 else { return nil }
        let points = (0..<numberOfTouches).map { location(ofTouch: $0, in: marker) }
        return CGPoint(x: points.map(\.x).reduce(0, +) / CGFloat(points.count),
                       y: points.map(\.y).reduce(0, +) / CGFloat(points.count))
    }

    private func update(_ event: UIEvent?) {
        guard let centre = centroid() else {
            // Dropping back to one finger restarts the reference point, so
            // lifting one of two fingers does not register as a huge jump.
            lastCentroid = nil
            if state == .began || state == .changed {
                state = .ended
                onTwoFingers?(false)
            }
            return
        }

        if let previous = lastCentroid {
            let delta = CGSize(width: centre.x - previous.x, height: centre.y - previous.y)
            if delta != .zero {
                onPan?(delta)
                state = state == .possible ? .began : .changed
            }
        } else {
            state = .began
            // Said the moment the second finger lands, before any movement: the
            // stroke the first finger started has to be abandoned, not merely
            // stopped, and by the time there is a delta it is already a line.
            onTwoFingers?(true)
        }
        lastCentroid = centre
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        // Anything that did not land on the overlay is not part of this gesture.
        if let marker {
            for touch in touches where !marker.bounds.contains(touch.location(in: marker)) {
                ignore(touch, for: event)
            }
        }
        update(event)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        update(event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesEnded(touches, with: event)
        lastCentroid = nil
        if numberOfTouches <= 1 {
            state = .ended
            // Said here as well as in `reset`, and that is not belt and braces.
            // `reset` runs only when UIKit takes the recogniser back through
            // `.possible`, and a recogniser removed mid-gesture — which is every
            // time SwiftUI rebuilds this layer — is never taken anywhere. The
            // flag then stays raised for the rest of the session, and the drawing
            // layer answers every touch by throwing the stroke away: no marks, no
            // erasing, nothing, in silence.
            onTwoFingers?(false)
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesCancelled(touches, with: event)
        lastCentroid = nil
        state = .cancelled
        onTwoFingers?(false)
    }

    override func reset() {
        onTwoFingers?(false)
        super.reset()
        lastCentroid = nil
    }
}

/// Fits the two-finger pan over a view.
struct TwoFingerPanLayer: UIViewRepresentable {
    let onPan: (CGSize) -> Void
    /// Two fingers went down, or came back up.
    var onTwoFingers: (Bool) -> Void = { _ in }

    func makeUIView(context: Context) -> UIView {
        let view = GestureHostView()
        let recogniser = TwoFingerPanRecogniser(target: context.coordinator,
                                                action: #selector(Coordinator.noop))
        // Read through the coordinator, never captured: a closure built here
        // captures the first render's state and goes stale.
        recogniser.onPan = { [weak coordinator = context.coordinator] delta in
            coordinator?.onPan(delta)
        }
        recogniser.onTwoFingers = { [weak coordinator = context.coordinator] down in
            coordinator?.onTwoFingers(down)
        }
        recogniser.delegate = context.coordinator
        recogniser.marker = view
        view.recogniser = recogniser
        view.keepAlive = context.coordinator
        view.addGestureRecognizer(recogniser)
        return view
    }

    static func dismantleUIView(_ view: UIView, coordinator: Coordinator) {
        // Whatever this layer had raised goes down with it.
        coordinator.fingers(false)
        (view as? GestureHostView)?.detach()
    }

    func updateUIView(_ view: UIView, context: Context) {
        context.coordinator.handler = onPan
        context.coordinator.fingers = onTwoFingers
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(handler: onPan, fingers: onTwoFingers)
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var handler: (CGSize) -> Void
        var fingers: (Bool) -> Void
        init(handler: @escaping (CGSize) -> Void, fingers: @escaping (Bool) -> Void) {
            self.handler = handler
            self.fingers = fingers
        }
        func onPan(_ delta: CGSize) { handler(delta) }
        func onTwoFingers(_ down: Bool) { fingers(down) }
        @objc func noop() {}

        /// Runs alongside whatever else is fitted — the pinch in particular,
        /// which claims every two-finger event, so without this nothing else
        /// would ever receive one.
        func gestureRecognizer(_ recogniser: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
            true
        }
    }
}
