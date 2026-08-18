package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.ceil

/**
 * [RenderScale] is the one function both the renderer and the prefetcher must
 * answer identically, so it is the highest-value thing in the Kotlin layer to
 * pin down. It is also pure — no Android types — so it runs here in
 * milliseconds instead of waiting on a device MIUI may refuse to install to.
 *
 * The sweep below is deliberately a loop over real page geometries rather than
 * a handful of hand-picked cases: the bugs in this area have all been "works
 * for A4, wrong for the landscape page on sheet 40".
 */
class RenderScaleTest {

    private val pages = listOf(
        "A4 portrait" to PageSize(595f, 842f),
        "A4 landscape" to PageSize(842f, 595f),
        "A3 portrait" to PageSize(842f, 1191f),
        "A0 poster" to PageSize(2384f, 3370f),
        "US Letter" to PageSize(612f, 792f),
        "square" to PageSize(500f, 500f),
        "wide banner" to PageSize(3000f, 200f),
        "tall strip" to PageSize(200f, 3000f),
    )

    private val viewportWidths =
        listOf(1f, 48f, 120f, 190f, 480f, 720f, 1080f, 1600f, 2400f, 8000f)

    /**
     * Mirrors `quantise_zoom` in `rust/pdf_core/src/render/cache.rs`.
     *
     * Duplicated on purpose. The point of the contract test below is to fail
     * when the two implementations drift, so it has to hold its own copy of what
     * Rust does — asking Rust at test time would just make the test agree with
     * whatever Rust currently is.
     */
    private fun rustEffectiveZoom(scale: Float): Float {
        val clamped = maxOf(scale, RenderScale.QUANTUM)
        val steps = ceil(clamped / RenderScale.QUANTUM)
        return maxOf(steps, 1f) * RenderScale.QUANTUM
    }

    /**
     * The cross-language contract, and the only test here that guards a bug that
     * has already happened: prefetch and render disagreed about scale, so the
     * cache never once produced a hit in normal use.
     *
     * Kotlin picks the scale and allocates the bitmap; Rust quantises that same
     * scale into a cache key. If Rust would round it up, the key stands for a
     * different pixel size than the bitmap Kotlin just allocated, every lookup
     * is a size mismatch, and the cache silently degrades to a no-op.
     */
    @Test
    fun rustNeverRequantisesAScaleKotlinProduced() {
        for ((name, page) in pages) {
            for (width in viewportWidths) {
                val scale = RenderScale.forPage(page, width)
                assertEquals(
                    "$name at ${width}px: Kotlin chose $scale but Rust would key it as " +
                        "${rustEffectiveZoom(scale)} — every lookup would miss on size",
                    scale,
                    rustEffectiveZoom(scale),
                    0f,
                )
            }
        }
    }

    @Test
    fun everyScaleLandsOnAQuantumBoundary() {
        for ((name, page) in pages) {
            for (width in viewportWidths) {
                val scale = RenderScale.forPage(page, width)
                val steps = scale / RenderScale.QUANTUM
                assertEquals(
                    "$name at ${width}px: $scale is not a whole number of quantum steps",
                    steps.toDouble(),
                    Math.round(steps).toDouble(),
                    1e-4,
                )
            }
        }
    }

    /**
     * MAX_PIXELS is documented as the ceiling that keeps `Bitmap.createBitmap`
     * from failing at extreme zoom — a hard guarantee, not a hint.
     */
    @Test
    fun theSixteenMegapixelCeilingIsActuallyEnforced() {
        for ((name, page) in pages) {
            for (width in viewportWidths) {
                val scale = RenderScale.forPage(page, width)
                val (pixelWidth, pixelHeight) = page.pixelSize(scale)
                val pixels = pixelWidth.toLong() * pixelHeight
                assertTrue(
                    "$name at ${width}px: scale $scale gives ${pixelWidth}x$pixelHeight " +
                        "= $pixels px, over the ${RenderScale.MAX_PIXELS.toLong()} ceiling",
                    pixels <= RenderScale.MAX_PIXELS.toLong(),
                )
            }
        }
    }

    @Test
    fun aWiderViewportNeverAsksForFewerPixels() {
        for ((name, page) in pages) {
            var previous = 0f
            for (width in viewportWidths) {
                val scale = RenderScale.forPage(page, width)
                assertTrue(
                    "$name: widening to ${width}px dropped the scale from $previous to $scale",
                    scale >= previous,
                )
                previous = scale
            }
        }
    }

    @Test
    fun degenerateInputsFallBackToTheSmallestStepRatherThanThrowing() {
        val page = PageSize(595f, 842f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(page, 0f), 0f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(page, -100f), 0f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(page, Float.NaN), 0f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(page, Float.POSITIVE_INFINITY), 0f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(PageSize(0f, 842f), 1080f), 0f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(PageSize(595f, 0f), 1080f), 0f)
        assertEquals(RenderScale.QUANTUM, RenderScale.forPage(PageSize(-5f, -5f), 1080f), 0f)
    }

    /**
     * Thumbnails must key off a fixed width, not the reading zoom — otherwise
     * every zoom step orphans the entire rail.
     */
    @Test
    fun thumbnailScaleDependsOnlyOnThePage() {
        for ((name, page) in pages) {
            val expected = RenderScale.forPage(page, RenderScale.THUMBNAIL_WIDTH_PX)
            assertEquals("$name", expected, RenderScale.thumbnailFor(page), 0f)
        }
    }

    @Test
    fun theProxyPassIsNeverMoreExpensiveThanTheReadablePass() {
        for ((name, page) in pages) {
            for (width in viewportWidths) {
                val proxy = RenderScale.proxyFor(page, width)
                val full = RenderScale.forPage(page, width)
                assertTrue(
                    "$name at ${width}px: proxy $proxy exceeds the full render $full",
                    proxy <= full,
                )
            }
        }
    }

    @Test
    fun aPageNeverRoundsAwayToNothing() {
        val hairline = PageSize(0.4f, 0.4f)
        val (w, h) = hairline.pixelSize(RenderScale.forPage(hairline, 1080f))
        assertTrue("a page must always occupy at least one pixel", w >= 1 && h >= 1)
    }
}
