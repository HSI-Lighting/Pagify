import SwiftUI
import UIKit

/// One- and two-finger tracking for the model and drawing viewers: a
/// one-finger drag reports its per-frame delta, a two-finger drag reports
/// its own (averaged) delta plus a pinch ratio and midpoint — the same
/// three gesture categories Android's own `pointerInput`/`awaitEachGesture`
/// blocks report in `ModelViewer.kt`/`DrawingViewer.kt`.
///
/// Built as a raw touch tracker rather than SwiftUI's `DragGesture`/
/// `MagnificationGesture` for the same reason Android's is raw: **finger
/// count is the whole feature here** (one finger orbits a model, two pans
/// and pinches it), and SwiftUI's high-level gestures don't expose it —
/// `DragGesture` reports a translation regardless of how many fingers are
/// down, and there is no way to ask it "was that one touch or two."
final class TrackballGestureView: UIView {
    /// A drag's per-frame delta, in points, plus how many fingers made it.
    var onDrag: ((_ dx: CGFloat, _ dy: CGFloat, _ fingers: Int) -> Void)?
    /// A pinch's frame-to-frame distance ratio (>1 = fingers moved apart)
    /// and the screen-space midpoint between the two fingers.
    var onPinch: ((_ ratio: CGFloat, _ midpoint: CGPoint) -> Void)?
    /// Every finger has lifted, with no drag having moved — a tap.
    var onTap: ((CGPoint) -> Void)?
    /// Every finger has lifted. Nothing to do with success or failure; it's
    /// where a caller drops a "gesturing" flag for proxy-resolution redraws.
    var onGestureEnded: (() -> Void)?

    private var lastPoint: [UITouch: CGPoint] = [:]
    private var lastPinchDistance: CGFloat?
    private var moved = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        backgroundColor = .clear
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        isMultipleTouchEnabled = true
        backgroundColor = .clear
    }

    private var active: [UITouch] { Array(lastPoint.keys) }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        for touch in touches { lastPoint[touch] = touch.location(in: self) }
        if active.count == 2 {
            lastPinchDistance = distance(active[0], active[1])
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        let current = active
        switch current.count {
        case 1:
            let touch = current[0]
            let now = touch.location(in: self)
            let prev = lastPoint[touch] ?? now
            let dx = now.x - prev.x
            let dy = now.y - prev.y
            if dx != 0 || dy != 0 {
                moved = true
                onDrag?(dx, dy, 1)
            }
            lastPoint[touch] = now

        case let count where count >= 2:
            let a = current[0]
            let b = current[1]
            let nowA = a.location(in: self)
            let nowB = b.location(in: self)
            let prevA = lastPoint[a] ?? nowA
            let prevB = lastPoint[b] ?? nowB

            let dist = distance(a, b)
            if let last = lastPinchDistance, last > 0, dist > 0 {
                let midpoint = CGPoint(x: (nowA.x + nowB.x) / 2, y: (nowA.y + nowB.y) / 2)
                onPinch?(dist / last, midpoint)
            }
            lastPinchDistance = dist

            let dx = ((nowA.x - prevA.x) + (nowB.x - prevB.x)) / 2
            let dy = ((nowA.y - prevA.y) + (nowB.y - prevB.y)) / 2
            if dx != 0 || dy != 0 {
                moved = true
                onDrag?(dx, dy, 2)
            }
            for touch in current { lastPoint[touch] = touch.location(in: self) }

        default:
            break
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        if !moved, touches.count == 1, let touch = touches.first {
            onTap?(touch.location(in: self))
        }
        for touch in touches { lastPoint.removeValue(forKey: touch) }
        if active.count < 2 { lastPinchDistance = nil }
        if active.isEmpty {
            onGestureEnded?()
            moved = false
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        touchesEnded(touches, with: event)
    }

    private func distance(_ a: UITouch, _ b: UITouch) -> CGFloat {
        let pa = a.location(in: self)
        let pb = b.location(in: self)
        return (pa.x - pb.x).magnitude > 0 || (pa.y - pb.y).magnitude > 0
            ? hypot(pa.x - pb.x, pa.y - pb.y)
            : 0
    }
}

/// SwiftUI's own way in. Configured once per screen through the three
/// closures below rather than exposing `TrackballGestureView` itself, so
/// each viewer states its own gesture-to-camera mapping at the call site
/// instead of reaching into the view.
struct TrackballGestureSurface: UIViewRepresentable {
    var onDrag: (_ dx: CGFloat, _ dy: CGFloat, _ fingers: Int) -> Void
    var onPinch: (_ ratio: CGFloat, _ midpoint: CGPoint) -> Void
    var onTap: (CGPoint) -> Void = { _ in }
    var onGestureEnded: () -> Void = {}

    func makeUIView(context: Context) -> TrackballGestureView {
        let view = TrackballGestureView()
        view.onDrag = onDrag
        view.onPinch = onPinch
        view.onTap = onTap
        view.onGestureEnded = onGestureEnded
        return view
    }

    func updateUIView(_ view: TrackballGestureView, context: Context) {
        view.onDrag = onDrag
        view.onPinch = onPinch
        view.onTap = onTap
        view.onGestureEnded = onGestureEnded
    }
}
