import SwiftUI
import UIKit

/// A scrollable rail of page thumbnails. Android's
/// `ui/components/ThumbnailStrip.kt`.
///
/// This exists because browsing and reading are different jobs with wildly
/// different costs. Finding a page only needs it to be *recognisable*: a
/// thumbnail is a fraction of a readable page, so a whole document can be
/// flicked through for roughly the price of one full render.
///
/// Tapping a thumbnail jumps the reader there, at which point that one page — and
/// only that page — is rendered at full resolution.
struct ThumbnailRail: View {
    let document: PagifyDocument
    let revision: Int
    let currentPage: Int
    /// Incremented by the reader when **you** scroll it; the rail follows only then.
    let followTick: Int
    let onSelectPage: (Int) -> Void

    @Environment(\.colorScheme) private var scheme

    /// Set once you drag the rail yourself, cleared when the reader moves.
    ///
    /// Browsing the rail ahead of the page you are reading is the whole point of
    /// having one, so the follow-the-reader behaviour below has to yield the
    /// moment you take over — otherwise the rail drags you back and is unusable.
    /// Driven by an actual drag, not by "is scrolling", which is also true during
    /// the rail's own programmatic scrolling.
    @State private var userIsBrowsing = false
    @State private var visible: Set<Int> = []
    /// Held so a tap can bring its own page to the middle.
    @State private var commander = ScrollCommander()

    /// Needed to predict where the page number under each thumbnail will land.
    @Environment(\.displayScale) private var displayScale

    /// Shared with the reader, which must subtract it when measuring pages.
    static let width: CGFloat = 104

    var body: some View {
        ScrollView(.vertical, showsIndicators: false) {
            LazyVStack(spacing: RailMetrics.spacing) {
                ForEach(0..<document.pageCount, id: \.self) { index in
                    cell(index)
                }
            }
            .padding(.vertical, RailMetrics.spacing)
            .padding(.horizontal, RailMetrics.inset)
            .background(ScrollObserver(onFound: { commander.scrollView = $0 }))
        }
        .simultaneousGesture(
            DragGesture(minimumDistance: 6).onChanged { _ in userIsBrowsing = true }
        )
        // Follow the reader, bringing the current page to the **middle** of
        // the rail.
        //
        // Centred is a deliberate departure from Android, which anchors this
        // to the top. What is *not* departed from is the trigger: the rail
        // still follows the tick and never watches `currentPage`. That is the
        // part carrying the bug — tapping a thumbnail centres the page in the
        // reader, which leaves the *previous* page topmost there and echoes
        // back as a page change indistinguishable from a real one. A rail
        // watching `currentPage` would chase that echo.
        //
        // Keyed on `followTick`, which the reader increments *only* when you
        // scroll it yourself — never when a page is chosen from here. Tapping
        // a thumbnail must leave the rail exactly where it is, anywhere in the
        // document: you were looking at a particular run of pages, and moving
        // the rail out from under you loses your place and makes picking a
        // second page from that run needlessly awkward.
        //
        // Driven by that explicit signal rather than by watching `currentPage`,
        // because centring the chosen page leaves the *previous* page topmost
        // in the reader, which echoes back as a page change and is
        // indistinguishable from a real one.
        .onChange(of: followTick) { _, tick in
            guard tick != 0 else { return }
            userIsBrowsing = false
            guard (0..<document.pageCount).contains(currentPage) else { return }
            // Animated only when the page is already on the rail.
            //
            // A follow from across the document builds and discards every cell
            // in between, and the correction below fires on the first of them —
            // the page it is looking for is off-rail by definition — replacing
            // the animation with an unanimated set to the offset it was already
            // travelling to. The jump is what happened either way; asking for it
            // outright is what stops it happening halfway through an animation.
            centre(on: currentPage, animated: visible.contains(currentPage))
        }
        // ...and once more if the page being read is still not on screen.
        //
        // With the offset computed rather than estimated this should not have
        // to fire, and when it does it lands and stops. It stays because the
        // arithmetic below trusts the engine's page sizes to match the cells'
        // own layout, and a correction that converges is a cheap way of being
        // wrong about that safely.
        .onChange(of: visible) { _, shown in
            guard !userIsBrowsing, !shown.isEmpty,
                  (0..<document.pageCount).contains(currentPage),
                  !shown.contains(currentPage) else { return }
            centre(on: currentPage, animated: false)
        }
        .frame(width: Self.width)
        .background(PagifyColor.surfaceVariant(scheme))
    }

    @ViewBuilder
    private func cell(_ index: Int) -> some View {
        PageThumbnail(document: document,
                      index: index,
                      revision: revision,
                      isCurrent: index == currentPage,
                      onSelect: {
                          // The chosen page is brought to the middle, so the
                          // selected thumbnail is centred however it was chosen.
                          //
                          // A departure from Android, which leaves the rail
                          // exactly where it is on a tap — and from the reasoning
                          // given for `followTick` above, which is Android's.
                          // Asked for directly: a page chosen from the rail was
                          // being left at its bottom edge, and the middle is
                          // where it was wanted.
                          //
                          // Centred by arithmetic, not by `scrollTo`. A rail cell
                          // a hundred pages down has never been built, so SwiftUI
                          // can only *estimate* where it is, and the estimate is
                          // wide enough to drop the chosen thumbnail at the bottom
                          // edge of the rail instead of its middle.
                          onSelectPage(index)
                          centre(on: index, animated: true)
                      })
            .id(index)
            .onAppear { visible.insert(index) }
            .onDisappear { visible.remove(index) }
    }

    /// Put `index` in the middle of the rail.
    ///
    private func centre(on index: Int, animated: Bool) {
        guard let scroll = commander.scrollView, scroll.bounds.height > 0 else { return }
        let extent = scroll.bounds.height
        let height = RailMetrics.height(index, of: document, scale: displayScale)
        let top = RailMetrics.top(index, of: document, scale: displayScale)
        SessionRecorder.shared.record("NAVIGATION",
            String(format: "rail centre page=%d top=%.0f h=%.0f extent=%.0f offset=%.0f "
                         + "| content=%.0f computed=%.0f inset=%.0f/%.0f",
                   index, top, height, extent, scroll.contentOffset.y,
                   scroll.contentSize.height,
                   RailMetrics.top(document.pageCount, of: document, scale: displayScale),
                   scroll.adjustedContentInset.top, scroll.adjustedContentInset.bottom))
        commander.scroll(toContentY: top - max(0, (extent - height) / 2), animated: animated)
    }
}

/// Where each rail cell sits, without asking the rail.
///
/// The cells are laid out from the engine's page sizes, so the same sizes
/// reproduce their positions exactly — no measuring, and nothing to invalidate.
///
/// Asked of the engine every time, and never kept. A height held by index
/// outlives the reorder, delete, insert or rotation that renumbered or reshaped
/// the page under it, and because a jump sums every cell above the target, one
/// stale entry displaces every page below it. A rotation would not even be seen:
/// it names its own page, so only that page's revision moves and the rail's
/// never does. `pagify_get_page_size` loads nothing, and the sum is paid only
/// when a jump is made.
enum RailMetrics {
    /// The gap between cells, and the padding above the first one.
    static let spacing: CGFloat = 10
    /// The rail's own horizontal padding.
    static let inset: CGFloat = 8
    /// What is left for a cell across the rail.
    static var cellWidth: CGFloat { ThumbnailRail.width - inset * 2 }

    /// The page number under each thumbnail: its top padding plus one line of it.
    ///
    /// The line is the one SwiftUI will lay out, not the one the font asks for.
    /// `Text` rounds its height up to a whole device pixel, so a raw `lineHeight`
    /// is a fraction of a point short in every single cell — nothing on its own,
    /// but `top(_:of:scale:)` sums it, and by the end of a long document that is
    /// most of a thumbnail's worth of drift downwards.
    static func captionHeight(scale: CGFloat) -> CGFloat {
        let line = UIFont.preferredFont(forTextStyle: .caption2).lineHeight
        let grid = max(scale, 1)
        return 3 + (line * grid).rounded(.up) / grid
    }

    static func height(_ index: Int, of document: PagifyDocument, scale: CGFloat) -> CGFloat {
        let size = (try? document.pageSize(index)) ?? CGSize(width: 595, height: 842)
        let aspect = size.width > 0 && size.height > 0
            ? size.width / size.height
            : PageThumbnail.defaultAspect
        return cellWidth / max(aspect, 0.01) + captionHeight(scale: scale)
    }

    static func top(_ index: Int, of document: PagifyDocument, scale: CGFloat) -> CGFloat {
        var y = spacing
        for page in 0..<max(0, index) { y += height(page, of: document, scale: scale) + spacing }
        return y
    }
}
