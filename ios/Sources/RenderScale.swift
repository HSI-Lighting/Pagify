import CoreGraphics
import Foundation

/// Choosing what scale to rasterise a page at.
///
/// Every render goes through here, for two reasons the app cannot see and the
/// engine cannot fix on its own.
///
/// **The cache is keyed on a quantised zoom.** A pinch produces a continuum of
/// float scales, and asking for 2.0413 then 2.0417 fills the engine's LRU with
/// near-duplicates while missing on every frame. Quantising here — to the same
/// step the engine uses — is what makes the cache a cache.
///
/// **Nothing else caps the size.** A full page at an unbounded zoom is an
/// arbitrarily large allocation, and the engine's own ceiling refuses it rather
/// than trimming it, so the page simply fails to draw at the moment the reader
/// zoomed in to look at it.
enum RenderScale {
    /// Must equal `ZOOM_QUANTUM` in `rust/pdf_core/src/render/cache.rs`. If the
    /// two ever disagree the cache silently stops hitting.
    static let quantum: CGFloat = 0.25

    /// The most pixels worth rasterising for one page.
    static let maxPixels: CGFloat = 16_000_000

    /// The first, cheap pass while a zoom is still moving. Measured on Android at
    /// about 5 ms against 50–99 ms for the readable pass.
    static let proxyFraction: CGFloat = 0.25

    /// What the rail asks for.
    static let thumbnailWidthPx: CGFloat = 190

    /// The scale to render `pageSize` at to fill `targetPixelWidth`.
    static func forPage(_ pageSize: CGSize, targetPixelWidth: CGFloat) -> CGFloat {
        guard pageSize.width > 0, pageSize.height > 0, targetPixelWidth > 0 else {
            return quantum
        }

        let ideal = targetPixelWidth / pageSize.width

        // The area ceiling, expressed as the largest scale that still fits.
        let maxScale = sqrt(maxPixels / (pageSize.width * pageSize.height))

        // Rounded *up*, so the raster is never lower-resolution than asked for —
        // scaling a bitmap down is free and sharp, scaling up is neither.
        var stepped = (min(ideal, maxScale) / quantum).rounded(.up) * quantum

        // Rounding up can push it back over the ceiling; step down once if so.
        if stepped > maxScale { stepped -= quantum }

        return max(stepped, quantum)
    }

    /// The scale a thumbnail wants.
    ///
    /// A rail thumbnail is a fixed pixel width whatever the page's shape, so it
    /// goes through the same quantiser as everything else — a rail of ten pages
    /// asking ten unquantised scales is ten cache misses on every scroll.
    static func thumbnailFor(_ pageSize: CGSize,
                             targetPixelWidth: CGFloat = thumbnailWidthPx) -> CGFloat {
        forPage(pageSize, targetPixelWidth: targetPixelWidth)
    }

    /// The cheap pass, for while the zoom is still moving.
    static func proxyFor(_ pageSize: CGSize, targetPixelWidth: CGFloat) -> CGFloat {
        forPage(pageSize, targetPixelWidth: targetPixelWidth * proxyFraction)
    }
}

/// How far the reader can zoom, and what the gestures snap to.
enum Zoom {
    static let minimum: CGFloat = 1
    static let maximum: CGFloat = 8
    /// The page fits the width of the reader.
    static let fitWidth: CGFloat = 1
    static let doubleTap: CGFloat = 2.5
    /// How much of a pinch it takes, in the page list, before the reader is
    /// handed over to the zoomed view.
    static let pinchHandover: CGFloat = 1.12

    static func clamp(_ scale: CGFloat) -> CGFloat {
        min(max(scale, minimum), maximum)
    }
}


/// Admits a bounded number of full-page rasters at once.
///
/// A recording of a scroll through a 149-page catalogue caught **four** page
/// renders finishing on the same millisecond, 280ms each: four PDFium rasters
/// competing for the same cores, and the scroll stuttering behind them. Over half
/// the stalls in that session had a render landing inside them.
///
/// Nothing here was bounded — each row started its own `Task.detached`. Android
/// does not have the problem in the same way: its page renders go to
/// `Dispatchers.Default`, which is already bounded by core count, and its
/// thumbnails hold a `Semaphore(1)`. iOS matched Android on thumbnails —
/// `ThumbnailRenderer` is an actor, so one at a time — and matched nothing at all
/// on pages.
///
/// Two, not one: the page being read and the one arriving behind it can both be
/// working without leaving the scroll to fight them for a core.
actor RenderGate {
    static let shared = RenderGate(limit: 2)

    private let limit: Int
    private var running = 0
    private var waiting: [CheckedContinuation<Void, Never>] = []

    init(limit: Int) { self.limit = limit }

    /// Run `body` once there is room, and make room again afterwards.
    nonisolated func run<T>(_ body: () async -> T) async -> T {
        await enter()
        let result = await body()
        await leave()
        return result
    }

    private func enter() async {
        if running < limit {
            running += 1
            return
        }
        await withCheckedContinuation { waiting.append($0) }
    }

    private func leave() {
        // Handed straight to whoever is next, so the count never dips and lets a
        // third render in behind them.
        if waiting.isEmpty {
            running -= 1
        } else {
            waiting.removeFirst().resume()
        }
    }
}
