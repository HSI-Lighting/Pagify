import Foundation
import SwiftUI

/// The one place a page thumbnail is drawn, shared by the rail, the organiser
/// grid and the navigator's map.
///
/// A serial queue rather than a pool of tasks. The organiser asks for every page
/// of the document at once, and a hundred simultaneous renders means a hundred
/// PDFium page objects alive together — which is how a 149-page document ran the
/// device out of memory while merely opening a sheet. Serialising also makes the
/// cancellation check below worth having: a cell that scrolled away before its
/// turn came round is skipped instead of drawn for nobody, so flicking through a
/// long rail costs the pages you stopped on rather than every page you passed.
final class ThumbnailRenderer: @unchecked Sendable {
    static let shared = ThumbnailRenderer()

    private let queue = DispatchQueue(label: "com.hsilighting.pagify.thumbnails", qos: .utility)

    /// The page's size in points. It loads no page content, but it is still an
    /// engine call, so it queues behind the renders rather than racing them.
    func size(of document: PagifyDocument, page: Int) async -> CGSize? {
        let cancelled = Cancellation()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<CGSize?, Never>) in
                queue.async {
                    guard !cancelled.isSet else { return continuation.resume(returning: nil) }
                    continuation.resume(returning: try? document.pageSize(page))
                }
            }
        } onCancel: {
            cancelled.set()
        }
    }

    func image(of document: PagifyDocument, page: Int, scale: CGFloat) async -> CGImage? {
        let cancelled = Cancellation()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<CGImage?, Never>) in
                queue.async {
                    guard !cancelled.isSet else { return continuation.resume(returning: nil) }
                    continuation.resume(returning: try? document.render(page: page, scale: scale))
                }
            }
        } onCancel: {
            cancelled.set()
        }
    }

    /// A flag the queue can read without reaching back into the task that set it.
    private final class Cancellation: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false

        var isSet: Bool {
            lock.lock()
            defer { lock.unlock() }
            return value
        }

        func set() {
            lock.lock()
            value = true
            lock.unlock()
        }
    }
}

/// What a thumbnail is a picture of.
///
/// Both halves matter. Rotating page 3 changes how it draws without changing
/// which page it is, and deleting page 3 changes what page 4 *is* without
/// changing either count — key on one alone and the cell keeps showing the
/// picture it had before the edit.
struct PageRenderKey: Hashable {
    let index: Int
    let revision: Int
}

/// One page of the rail.
///
/// Deliberately *not* shared with the organiser's cell. The two are different
/// strategies — this one is a jump target sized to the page's own shape, the
/// other is a fixed grid slot carrying four buttons — and a single component
/// serving both grew a `box` parameter that neither of them wanted.
struct PageThumbnail: View {
    let document: PagifyDocument
    let index: Int
    let revision: Int
    var isCurrent: Bool = false
    let onSelect: () -> Void

    @Environment(\.colorScheme) private var scheme
    @Environment(\.displayScale) private var displayScale
    @State private var image: CGImage?
    /// The cell's shape, known before it is laid out.
    ///
    /// A cell that starts at a guessed A4 shape and corrects when its raster
    /// arrives changes its own height, which moves every cell below it — so the
    /// page the rail is trying to follow slides back out of view, the drift
    /// correction scrolls after it, and that moves more cells. With cells built
    /// lazily the correction never converges: the rail scrolls for ever, a third
    /// of a point at a time. Asking the engine first costs one cheap call and the
    /// shape is then never wrong.
    @State private var aspect: CGFloat

    /// A4 upright, until the real dimensions land. Cells have to occupy space
    /// before they can be measured, and a guess close to the common case is what
    /// keeps the rail from shuffling as it fills.
    static let defaultAspect: CGFloat = 595.0 / 842.0

    init(document: PagifyDocument, index: Int, revision: Int,
         isCurrent: Bool, onSelect: @escaping () -> Void) {
        self.document = document
        self.index = index
        self.revision = revision
        self.isCurrent = isCurrent
        self.onSelect = onSelect
        let size = (try? document.pageSize(index)) ?? CGSize(width: 595, height: 842)
        _aspect = State(initialValue: size.width > 0 && size.height > 0
                        ? size.width / size.height
                        : Self.defaultAspect)
    }

    var body: some View {
        // The whole cell is the target, page number included — a 104pt-wide rail
        // is already a small thing to hit, and the number reads as part of the
        // cell rather than as a caption beside it.
        Button(action: onSelect) {
            VStack(spacing: 0) {
                ZStack {
                    Rectangle().fill(PagifyColor.surface(scheme))
                    if let image {
                        Image(decorative: image, scale: displayScale)
                            .resizable()
                            .scaledToFit()
                    }
                }
                .aspectRatio(max(aspect, 0.01), contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 3))
                .overlay(
                    RoundedRectangle(cornerRadius: 3)
                        .strokeBorder(isCurrent ? PagifyColor.primary(scheme) : Color(.separator),
                                      lineWidth: isCurrent ? 2 : 1)
                )

                Text("\(index + 1)")
                    .font(.caption2)
                    .fontWeight(isCurrent ? .bold : .regular)
                    .foregroundStyle(isCurrent ? PagifyColor.primary(scheme) : Color.secondary)
                    .padding(.top, 3)
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Go to page \(index + 1)")
        .task(id: PageRenderKey(index: index, revision: revision)) {
            // Cleared before anything is awaited. Keeping the previous picture up
            // while the new one renders is how a rotated page went on showing its
            // old orientation until the rail was scrolled away and back.
            image = nil

            guard let size = await ThumbnailRenderer.shared.size(of: document, page: index),
                  size.width > 0, size.height > 0 else { return }
            image = await ThumbnailRenderer.shared.image(of: document, page: index,
                                                         scale: RenderScale.thumbnailFor(size))
        }
    }
}

/// The visible region of the current page, in page-relative fractions (0...1).
///
/// Fractions rather than pixels so the navigator never needs to know about zoom
/// levels, screen scales or scroll ranges — the reader resolves all of that once.
struct ViewportWindow: Equatable {
    let left: CGFloat
    let top: CGFloat
    let width: CGFloat
    let height: CGFloat

    /// True when the whole page is visible, i.e. there is nothing to navigate.
    var coversEverything: Bool { width >= 0.999 && height >= 0.999 }

    static let full = ViewportWindow(left: 0, top: 0, width: 1, height: 1)
}

/// A minimap of the current page showing which part of it is on screen.
///
/// Belongs on screen only while zoomed in: at fit-width it would always show a
/// full-page rectangle and earn nothing, which is what `coversEverything` is for.
/// Tapping or dragging inside it recentres the viewport, far quicker than
/// repeatedly panning at high zoom.
struct PageNavigator: View {
    let document: PagifyDocument
    let pageIndex: Int
    /// The page's size in points, or nil while it is still being measured.
    let pageSize: CGSize?
    let window: ViewportWindow
    let onRecenter: (CGFloat, CGFloat) -> Void
    /// Folded away to its handle. Remembered across documents.
    let minimized: Bool
    let onMinimized: (Bool) -> Void
    /// Where the folded handle sits, as fractions of the free space.
    let handlePosition: CGPoint
    let onHandleMoved: (CGPoint) -> Void
    var width: CGFloat = 88

    @Environment(\.displayScale) private var displayScale
    @State private var image: CGImage?

    private var aspect: CGFloat {
        guard let pageSize, pageSize.width > 0, pageSize.height > 0 else { return 0.7 }
        return pageSize.width / pageSize.height
    }

    private var mapSize: CGSize {
        CGSize(width: width, height: width / max(aspect, 0.01))
    }

    var body: some View {
        if minimized {
            ViewfinderHandle(position: handlePosition,
                             onMoved: onHandleMoved,
                             onOpen: { onMinimized(false) })
        } else {
            // An outer stack so the fold-away button can sit on the map's corner,
            // which is where someone who wants rid of it is already looking.
            ZStack(alignment: .topTrailing) {
                map
                foldButton
            }
        }
    }

    private var map: some View {
        ZStack {
            if let image {
                Image(decorative: image, scale: displayScale)
                    .resizable()
                    .scaledToFit()
            }
        }
        .frame(width: mapSize.width, height: mapSize.height)
        .background(Color.white.opacity(0.92))
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .overlay { indicator.allowsHitTesting(false) }
        .contentShape(Rectangle())
        // One recogniser for both gestures: a zero-distance drag reports its
        // first touch immediately, so a tap and a drag are the same event stream
        // and the map does not need to guess which one is starting.
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { value in emit(value.location) }
        )
        .accessibilityLabel("Page \(pageIndex + 1) navigator")
        .padding(8)
        .task(id: MapKey(page: pageIndex,
                         width: pageSize?.width ?? 0,
                         height: pageSize?.height ?? 0)) {
            image = nil
            guard let pageSize, pageSize.width > 0 else { return }
            // Its own scale rather than the rail's, so the map keeps a cache key
            // of its own and rendering it never evicts the strip.
            let scale = max(Self.mapWidthPixels / pageSize.width, 0.25)
            image = await ThumbnailRenderer.shared.image(of: document, page: pageIndex, scale: scale)
        }
    }

    private var indicator: some View {
        Canvas { context, size in
            let rect = CGRect(x: window.left * size.width,
                              y: window.top * size.height,
                              width: window.width * size.width,
                              height: window.height * size.height)

            // Dim what is off screen as four bands around the viewport rather
            // than one full-cover rectangle punched through: the punch needs its
            // own layer to composite, and without one it paints solid black over
            // the whole map.
            let dim = Color.black.opacity(0.3)
            context.fill(Path(CGRect(x: 0, y: 0, width: size.width, height: rect.minY)), with: .color(dim))
            context.fill(Path(CGRect(x: 0, y: rect.maxY,
                                     width: size.width, height: size.height - rect.maxY)),
                         with: .color(dim))
            context.fill(Path(CGRect(x: 0, y: rect.minY, width: rect.minX, height: rect.height)),
                         with: .color(dim))
            context.fill(Path(CGRect(x: rect.maxX, y: rect.minY,
                                     width: size.width - rect.maxX, height: rect.height)),
                         with: .color(dim))

            context.stroke(Path(rect), with: .color(Self.indicatorInk), lineWidth: 2)
        }
    }

    private var foldButton: some View {
        Button { onMinimized(true) } label: {
            Image(systemName: "arrow.down.right.and.arrow.up.left")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 24, height: 24)
                .background(Color.black.opacity(0.45), in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Fold the viewfinder away")
        .padding(2)
    }

    /// Map a touch inside the minimap to the page fraction it should centre on.
    private func emit(_ position: CGPoint) {
        guard mapSize.width > 0, mapSize.height > 0 else { return }
        onRecenter(min(max(position.x / mapSize.width, 0), 1),
                   min(max(position.y / mapSize.height, 0), 1))
    }

    /// Fixed map width in pixels; small enough that rendering it is negligible.
    private static let mapWidthPixels: CGFloat = 180

    private static let indicatorInk = Color(hex: 0x3F5F90)

    private struct MapKey: Hashable {
        let page: Int
        let width: CGFloat
        let height: CGFloat
    }
}

/// What is left when the viewfinder is folded away.
///
/// A handle rather than nothing at all: someone who folds it away in the middle
/// of a drawing has not decided never to see it again, and a control that
/// vanishes with no way back has to be looked for in Settings — which is exactly
/// the trip the fold-away button exists to save.
///
/// Draggable, because "somewhere else" is a different answer from "gone": the one
/// corner it starts in is over the page like everywhere else, and which part of
/// the page matters depends on the drawing.
///
/// `position` is fractions of the free space rather than points, so the handle
/// stays where it was put when the device is turned, and it is reported on the
/// way *up* rather than every frame — this ends in a file write.
private struct ViewfinderHandle: View {
    let position: CGPoint
    let onMoved: (CGPoint) -> Void
    let onOpen: () -> Void

    @State private var dragged: CGPoint?
    @State private var dragOrigin: CGPoint?

    private static let size: CGFloat = 32
    private static let margin: CGFloat = 8

    var body: some View {
        GeometryReader { geometry in
            let freeX = max(0, geometry.size.width - Self.size - Self.margin * 2)
            let freeY = max(0, geometry.size.height - Self.size - Self.margin * 2)
            let resting = CGPoint(x: Self.margin + position.x * freeX,
                                  y: Self.margin + position.y * freeY)
            let at = dragged ?? resting

            Image(systemName: "map")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(.white)
                .frame(width: Self.size, height: Self.size)
                .background(Color.black.opacity(0.45), in: RoundedRectangle(cornerRadius: 8))
                .contentShape(RoundedRectangle(cornerRadius: 8))
                .offset(x: at.x, y: at.y)
                // A tap and a drag on the same 32pt target. Both are attached
                // because the handle has two jobs and one of them — moving it —
                // is the reason it was folded away rather than dismissed.
                .onTapGesture { onOpen() }
                .gesture(
                    DragGesture()
                        .onChanged { value in
                            let origin = dragOrigin ?? at
                            dragOrigin = origin
                            dragged = CGPoint(
                                x: min(max(origin.x + value.translation.width, Self.margin),
                                       Self.margin + freeX),
                                y: min(max(origin.y + value.translation.height, Self.margin),
                                       Self.margin + freeY)
                            )
                        }
                        .onEnded { _ in
                            dragOrigin = nil
                            guard let landed = dragged else { return }
                            onMoved(CGPoint(x: freeX > 0 ? (landed.x - Self.margin) / freeX : 0,
                                            y: freeY > 0 ? (landed.y - Self.margin) / freeY : 0))
                        }
                )
                .accessibilityLabel("Show the viewfinder")
        }
        // Dropped whenever the stored position changes, so the drag and the
        // setting can never disagree about where the handle is.
        .onChange(of: position) { _, _ in dragged = nil }
    }
}
