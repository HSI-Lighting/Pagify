package com.hsilighting.pagify.core

import kotlin.math.ceil
import kotlin.math.sqrt

/**
 * The single definition of "how many pixels per PDF point do we rasterise at".
 *
 * This lives on its own because two callers need the *identical* answer: the page
 * view that renders what you see, and the prefetcher that warms the native cache
 * ahead of you. They previously computed it separately — and disagreed, one
 * passing a points-to-pixels scale and the other a fit-width magnification factor.
 * Every prefetch therefore landed under a cache key the renderer never looked up,
 * so the cache never once produced a hit in normal use. Sharing this function is
 * what makes that class of bug impossible rather than merely fixed.
 *
 * Note the distinction the names now keep straight:
 *  - **zoom** is user-facing magnification, 1.0 == fit to viewport width.
 *  - **render scale** is pixels per point, which is what the engine wants.
 */
object RenderScale {

    /** Must match `ZOOM_QUANTUM` in `rust/pdf_core/src/render/cache.rs`. */
    const val QUANTUM = 0.25f

    /**
     * Per-page bitmap ceiling: 16 MP, i.e. 64 MB at 4 bytes per pixel.
     *
     * A page is a single bitmap, so its cost grows with the square of zoom — an A4
     * page at 8x would be ~930 MB and would fail in `Bitmap.createBitmap` well
     * before the engine's own size guard could reject it. Past this ceiling the
     * bitmap is upscaled for display instead: softer at extreme zoom, but it
     * cannot bring the app down.
     */
    const val MAX_PIXELS = 16_000_000.0

    /**
     * Render scale for [pageSize] drawn [targetPixelWidth] pixels wide.
     *
     * Quantised *up* to [QUANTUM] steps to match the native cache's own
     * quantisation: without that, every pixel of pinch drift would be a fresh
     * cache key, and rounding down would leave the raster slightly too blurry.
     */
    /**
     * Fraction of the readable width a proxy render uses.
     *
     * A quarter of the width is a sixteenth of the pixels, which is the
     * difference between roughly 5 ms and roughly 80 ms per page. Recognisable
     * while scrolling, and never mistaken for the readable pass.
     */
    const val PROXY_FRACTION = 0.25f

    /** Width of a navigator/strip thumbnail, in pixels. */
    const val THUMBNAIL_WIDTH_PX = 190f

    /** The cheap first pass. See [PROXY_FRACTION]. */
    fun proxyFor(pageSize: PageSize, targetPixelWidth: Float): Float =
        forPage(pageSize, targetPixelWidth * PROXY_FRACTION)

    /**
     * Scale for a thumbnail, independent of zoom.
     *
     * Fixed width rather than a fraction of the reading size so that thumbnails
     * keep one cache key no matter how far the reader is zoomed in — otherwise
     * every zoom step would orphan the entire strip.
     */
    fun thumbnailFor(pageSize: PageSize): Float = forPage(pageSize, THUMBNAIL_WIDTH_PX)

    fun forPage(pageSize: PageSize, targetPixelWidth: Float): Float {
        if (pageSize.widthPoints <= 0f || pageSize.heightPoints <= 0f) return QUANTUM
        if (!targetPixelWidth.isFinite() || targetPixelWidth <= 0f) return QUANTUM

        val ideal = targetPixelWidth / pageSize.widthPoints
        val maxScale = sqrt(
            MAX_PIXELS / (pageSize.widthPoints.toDouble() * pageSize.heightPoints.toDouble()),
        ).toFloat()

        val bounded = ideal.coerceAtMost(maxScale)
        return (ceil((bounded / QUANTUM).toDouble()) * QUANTUM).toFloat().coerceAtLeast(QUANTUM)
    }
}
