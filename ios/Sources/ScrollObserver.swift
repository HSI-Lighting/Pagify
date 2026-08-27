import SwiftUI
import UIKit

/// Reports where a `ScrollView` has got to, without invalidating anything.
///
/// The reader used to learn this from a `GeometryReader` in every row, publishing
/// a preference the reader wrote back into view state. That write re-ran the body,
/// which rebuilt every visible page — fresh arrays and a dozen closures apiece —
/// and a recording caught it happening **forty-two times a second** through a
/// scroll. Nothing was wrong with the arithmetic; the cost was asking for it.
///
/// A scroll view already knows its offset and will say so without a single view
/// being rebuilt. `contentOffset` is observed directly, the callback runs outside
/// SwiftUI's update cycle, and what the reader does with it — advancing the page
/// it thinks you are on — publishes only when that page actually changes.
///
/// `iOS 18`'s `onScrollGeometryChange` does exactly this and would be tidier; the
/// project targets 17.
struct ScrollObserver: UIViewRepresentable {
    /// The vertical content offset, every time it moves. `nil` when nobody is
    /// listening, and then nothing is observed at all.
    var onScroll: ((CGFloat) -> Void)?
    /// Handed the scroll view once it is found, so the reader can command it.
    var onFound: (UIScrollView) -> Void = { _ in }

    func makeUIView(context: Context) -> UIView {
        let view = ScrollObserverView()
        view.onScroll = onScroll
        view.onFound = onFound
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        (view as? ScrollObserverView)?.onScroll = onScroll
        (view as? ScrollObserverView)?.onFound = onFound
    }

    static func dismantleUIView(_ view: UIView, coordinator: ()) {
        (view as? ScrollObserverView)?.stopObserving()
    }
}

private final class ScrollObserverView: UIView {
    var onScroll: ((CGFloat) -> Void)?
    var onFound: ((UIScrollView) -> Void)?
    private var observation: NSKeyValueObservation?

    /// Never a touch target. This view exists to find a scroll view and watch it.
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return stopObserving() }
        guard observation == nil, let scroll = enclosingScrollView else { return }
        onFound?(scroll)

        // The rail wants only the scroll view itself, so that it can be told
        // where to go. A callback that does nothing still runs on every frame of
        // every flick — don't register one that has nowhere to go.
        guard onScroll != nil else { return }

        // `.initial` so the reader knows where it starts, not only where it goes.
        observation = scroll.observe(\.contentOffset, options: [.initial, .new]) { [weak self] view, _ in
            self?.onScroll?(view.contentOffset.y)
        }
    }

    func stopObserving() {
        observation?.invalidate()
        observation = nil
    }

    deinit { observation?.invalidate() }

    private var enclosingScrollView: UIScrollView? {
        var candidate = superview
        while let view = candidate {
            if let scroll = view as? UIScrollView { return scroll }
            candidate = view.superview
        }
        return nil
    }
}

/// A box for geometry the view reads but must never be re-laid-out by.
///
/// Page frames feed the zoom hand-over and nothing else. Holding them in view
/// state made every measurement an invalidation of the view the pages sit in;
/// holding them in a reference makes reading them free.
final class PageFrameBox {
    var frames: [Int: CGRect] = [:]
}


/// Holds the reader's scroll view so a jump can be exact.
///
/// `ScrollViewReader.scrollTo` has to *estimate* where an unbuilt row of a
/// `LazyVStack` sits, and over a hundred pages that estimate lands nowhere near —
/// which is why choosing a thumbnail deep in a document showed a different page
/// than the one tapped. Every page's position is known arithmetically, so the
/// offset can simply be set.
final class ScrollCommander {
    weak var scrollView: UIScrollView?

    /// Put `y` at the top of the viewport, clamped to what the content allows.
    ///
    /// The clamp is retried, and that is not belt-and-braces. A `LazyVStack`
    /// reports a **provisional** content height until it has built the rows it is
    /// being asked to scroll past — so deep in a long document the target is
    /// clipped to a content size that does not exist yet, and the jump lands
    /// short, showing the page low on screen instead of in the middle. Rows build
    /// as the scroll proceeds and the true height arrives a moment later, so the
    /// ask is repeated until it is no longer being cut down.
    func scroll(toContentY y: CGFloat, animated: Bool, attempt: Int = 0) {
        guard let scrollView else { return }
        let limit = max(0, scrollView.contentSize.height - scrollView.bounds.height)
        let wanted = max(0, y)
        let target = min(wanted, limit)
        scrollView.setContentOffset(CGPoint(x: scrollView.contentOffset.x, y: target),
                                    animated: animated)

        // Only when the content was genuinely too short for the ask, and only a
        // few times: a page near the true end of the document is clamped for ever
        // and must not be chased.
        guard target < wanted - 1, attempt < 6 else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
            self?.scroll(toContentY: y, animated: false, attempt: attempt + 1)
        }
    }
}
