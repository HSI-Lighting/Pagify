import SwiftUI
import UIKit

/// Dragging a placed caption around the page.
///
/// A plain SwiftUI `DragGesture` cannot do this. Inside a scroll view it takes
/// **every** pan the moment it exists, so the page stops scrolling — and the
/// caption is only under the finger occasionally. What is needed is a recogniser
/// that decides in `touchesBegan`, before the scroll view has committed to
/// anything, and **fails itself** when the finger is not on a caption. A failed
/// recogniser hands the gesture straight back.
final class CaptionPanRecogniser: UIPanGestureRecognizer {
    /// Asked once, on the way down, in this view's coordinates.
    var shouldBegin: ((CGPoint) -> Bool)?

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        // A second finger anywhere means a pinch, and a pinch is never a move.
        //
        // Let go the instant one arrives. `maximumNumberOfTouches` only stops the
        // extra finger *joining* this pan — it does nothing about the pan already
        // running on the first one, which goes on dragging the caption out from
        // under the gesture and starves the magnify gesture of its pinch. Two
        // fingers never land on the same frame on a real hand, so this is the
        // whole difference between a pinch that scales and one that does nothing.
        if event.allTouches?.filter({ $0.phase != .ended && $0.phase != .cancelled })
            .count ?? 0 > 1 {
            state = (state == .began || state == .changed) ? .cancelled : .failed
            return
        }

        super.touchesBegan(touches, with: event)
        guard let touch = touches.first,
              shouldBegin?(touch.location(in: view)) == true else {
            state = .failed
            return
        }
    }
}

/// The gesture layer over one page.
struct CaptionMoveLayer: UIViewRepresentable {
    /// Whether a caption is at this point, in view coordinates.
    let captionAt: (CGPoint) -> Int32?
    /// The caption in hand. Only this one answers a drag: a caption is taken in
    /// hand by a tap first, and manipulated second. Without that rule a finger
    /// that lands on words while pinching or scrolling carries them off, which is
    /// the difference between a page you can read and one that rearranges itself
    /// under you.
    var selected: Int32?
    /// Two fingers are down somewhere on the page. The drag abandons at once and
    /// puts the caption back, so the pinch scales it instead of dragging it.
    var pinching: Bool = false
    /// Called for every movement, with the total offset since the finger went
    /// down, in view coordinates.
    let onMove: (Int32, CGSize) -> Void
    /// Called once when the finger lifts, with the final offset.
    let onFinish: (Int32, CGSize) -> Void
    /// The drag was abandoned — a second finger arrived, or the system took the
    /// gesture. Nothing is committed and the caption goes back where it was.
    var onCancel: (Int32) -> Void = { _ in }
    /// A caption was double-tapped: reopen its words.
    ///
    /// Carried here rather than on a layer of its own because this view is
    /// already hit-testable **only where a caption is**. A full-page tap surface
    /// would swallow the double tap that means "zoom".
    /// A caption was tapped once: take it in hand.
    var onTap: (Int32) -> Void = { _ in }
    var onDoubleTap: (Int32) -> Void = { _ in }

    func makeUIView(context: Context) -> UIView {
        let view = CaptionHitView()
        let coordinator = context.coordinator
        coordinator.captionAt = captionAt
        view.captionAt = { [weak coordinator] in coordinator?.captionAt($0) }

        let pan = CaptionPanRecogniser(target: coordinator,
                                       action: #selector(Coordinator.handle(_:)))
        // Read through the coordinator, never captured directly.
        //
        // A closure built here captures the `captionAt` of the *first* render —
        // which closes over an empty list of marks, because nothing has been
        // drawn yet. It then answers "no caption here" forever, the recogniser
        // fails every touch, and dragging a caption silently does nothing.
        pan.shouldBegin = { [weak coordinator] point in
            guard let coordinator,
                  let id = coordinator.captionAt(point),
                  // …and it is the one already in hand. An untouched caption is
                  // inert: the first finger on it can only ever select it.
                  id == coordinator.selected else { return false }
            coordinator.holding = id
            return true
        }
        // One finger only, and it never claims the touch.
        //
        // Without `maximumNumberOfTouches` the second finger of a pinch is
        // absorbed into the pan, so the magnify gesture never sees two fingers
        // and a selected caption cannot be scaled. And with `cancelsTouchesInView`
        // left on, recognising here tears the touch out from under SwiftUI —
        // which is why the layers that already work (`PinchToZoomLayer`,
        // `TwoFingerPanLayer`) both switch it off.
        pan.delegate = coordinator
        pan.maximumNumberOfTouches = 1
        pan.cancelsTouchesInView = false
        pan.delaysTouchesBegan = false
        pan.delaysTouchesEnded = false
        context.coordinator.pan = pan
        view.addGestureRecognizer(pan)

        let doubleTap = UITapGestureRecognizer(target: coordinator,
                                               action: #selector(Coordinator.handleDoubleTap(_:)))
        doubleTap.delegate = coordinator
        doubleTap.numberOfTapsRequired = 2
        doubleTap.cancelsTouchesInView = false
        doubleTap.delaysTouchesBegan = false
        view.addGestureRecognizer(doubleTap)

        // Selecting lives here too, because this layer is what a touch on a
        // caption actually reaches. It sits over the tool surface — it has to, or
        // a drag on words is read as a placement instead of a move — and a view
        // that is hit-testable keeps the touch: the SwiftUI gesture underneath
        // never sees it, so the tap that selects has to be raised here.
        //
        // Held back until the double tap has failed, so re-wording a caption does
        // not first flash it selected. That is what Compose's `detectTapGestures`
        // does when an `onDoubleTap` is supplied.
        let tap = UITapGestureRecognizer(target: coordinator,
                                         action: #selector(Coordinator.handleTap(_:)))
        tap.delegate = coordinator
        tap.numberOfTapsRequired = 1
        tap.require(toFail: doubleTap)
        tap.cancelsTouchesInView = false
        tap.delaysTouchesBegan = false
        view.addGestureRecognizer(tap)
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        context.coordinator.captionAt = captionAt
        context.coordinator.selected = selected

        // Toggling `isEnabled` is how UIKit cancels a recogniser mid-flight: it
        // fires `.cancelled`, which puts the caption back where it was picked up.
        // `.possible` is the RESTING state of an idle recogniser, not an
        // in-flight one — bouncing `isEnabled` on it cancelled a perfectly good
        // recogniser on every single invalidation of this view.
        if pinching, let pan = context.coordinator.pan,
           pan.state == .began || pan.state == .changed {
            pan.isEnabled = false
            pan.isEnabled = true
        }
        context.coordinator.onMove = onMove
        context.coordinator.onFinish = onFinish
        context.coordinator.onCancel = onCancel
        context.coordinator.onDoubleTap = onDoubleTap
        context.coordinator.onTap = onTap
    }

    func makeCoordinator() -> Coordinator {
        let coordinator = Coordinator(onMove: onMove, onFinish: onFinish)
        coordinator.selected = selected
        coordinator.onCancel = onCancel
        coordinator.onDoubleTap = onDoubleTap
        coordinator.onTap = onTap
        return coordinator
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        /// Yes to this app's own two-finger recognisers. **No** to everything else.
        ///
        /// With no delegate at all UIKit answers no to everyone, and the instant
        /// the caption pan began it starved every other recogniser in the touch's
        /// set — including the pinch fitted to an ancestor, which is why a pinch
        /// beginning near a caption never saw its second finger.
        ///
        /// Answering yes to *everything* is the opposite mistake, and a worse one:
        /// it says yes to the scroll view's own pan, so dragging a caption scrolled
        /// the document at the same time and the words slid away under the finger.
        /// Only the two-finger pair may run alongside a caption drag; the scroller
        /// must still lose, exactly as it did before there was a delegate at all.
        func gestureRecognizer(_ recogniser: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
            other is PinchRecogniser || other is TwoFingerPanRecogniser
        }

        var captionAt: (CGPoint) -> Int32? = { _ in nil }
        var selected: Int32?
        weak var pan: UIPanGestureRecognizer?
    /// Two fingers are down somewhere on the page. The drag abandons at once and
    /// puts the caption back, so the pinch scales it instead of dragging it.
    var pinching: Bool = false
        var onMove: (Int32, CGSize) -> Void
        var onFinish: (Int32, CGSize) -> Void
        var onCancel: (Int32) -> Void = { _ in }
        var holding: Int32?
        var onDoubleTap: (Int32) -> Void = { _ in }
        var onTap: (Int32) -> Void = { _ in }

        @objc func handleTap(_ tap: UITapGestureRecognizer) {
            guard tap.state == .ended,
                  let id = captionAt(tap.location(in: tap.view)) else { return }
            onTap(id)
        }

        @objc func handleDoubleTap(_ tap: UITapGestureRecognizer) {
            guard tap.state == .ended,
                  let id = captionAt(tap.location(in: tap.view)) else { return }
            onDoubleTap(id)
        }

        init(onMove: @escaping (Int32, CGSize) -> Void,
             onFinish: @escaping (Int32, CGSize) -> Void) {
            self.onMove = onMove
            self.onFinish = onFinish
            super.init()
        }

        @objc func handle(_ pan: UIPanGestureRecognizer) {
            guard let id = holding else { return }
            let translation = pan.translation(in: pan.view)
            let offset = CGSize(width: translation.x, height: translation.y)

            switch pan.state {
            case .changed:
                // No slop: the caption follows from the first movement, which is
                // what makes it feel picked up rather than dragged into life.
                onMove(id, offset)
            case .ended:
                onFinish(id, offset)
                holding = nil
            case .cancelled, .failed:
                // Put back, not committed. A drag that turned into a pinch never
                // meant to move anything, and leaving it where the one finger had
                // dragged it to would shift the caption every time it is scaled.
                onCancel(id)
                holding = nil
            default:
                break
            }
        }
    }
}

/// Hit-testable **only where a caption is**.
///
/// This is what lets the layer sit over the whole page without swallowing
/// anything. A view that fails hit-testing everywhere never feeds its own
/// recogniser; a view that passes everywhere eats every tap meant for the page
/// beneath. Answering per point does both jobs: the recogniser sees exactly the
/// touches that start on a caption, and every other touch falls through to
/// SwiftUI as though this layer were not here.
private final class CaptionHitView: UIView {
    var captionAt: ((CGPoint) -> Int32?)?

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        captionAt?(point) != nil
    }
}
