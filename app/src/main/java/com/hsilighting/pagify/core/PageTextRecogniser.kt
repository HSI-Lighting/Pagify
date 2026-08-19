package com.hsilighting.pagify.core

import android.graphics.Bitmap
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.text.TextRecognition
import com.google.mlkit.vision.text.latin.TextRecognizerOptions
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.math.min
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Reading text off a page that has none.
 *
 * Some documents carry no text at all. The HSI 2017 catalogue is the case that
 * forced this: 95 pages, and the whole 2.97 GB file contains exactly one font,
 * subset to the twelve characters of "www.hsilighting.com". Everything a reader
 * would call text on those pages is vector artwork — the type was converted to
 * outlines when the artwork was placed from Illustrator. It reads perfectly and
 * selects in nothing, because there is nothing there to select.
 *
 * The output is deliberately [TextSegment], the same type the engine produces for
 * a real text layer. Selection, highlighting and the reading-order interval in
 * [TextSelection] then work on a recognised page exactly as they do on a normal
 * one, with no branch anywhere above this.
 *
 * What this is not: it does not write anything back into the PDF. Recognised text
 * lives for the session. Stamping an invisible text layer into the file is a
 * separate job — it needs a font embedded in the document — and it is the thing
 * that would make the text survive a save and be findable by other software.
 */
class PageTextRecogniser {

    // Latin-only. The catalogue is English, and the general model is several times
    // the size for scripts this app has no evidence of needing yet.
    private val client by lazy { TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS) }

    /**
     * Recognise the text in a rendered page.
     *
     * @param bitmap the page, rendered at [pageSize] scaled by any factor — the
     *   scale is derived from the two rather than passed, so they cannot disagree.
     * @return runs in reading order. **Not sorted**: ML Kit returns blocks and
     *   lines in reading order already, and [TextSegment] documents that order as
     *   meaningful. Sorting them by position would break selection across columns
     *   in exactly the way the geometric-band approach did.
     */
    suspend fun recognise(bitmap: Bitmap, pageSize: PageSize): List<TextSegment> {
        val recognised = suspendCancellableCoroutine { continuation ->
            val task = client.process(InputImage.fromBitmap(bitmap, 0))
                .addOnSuccessListener { continuation.resume(it) }
                .addOnFailureListener { continuation.resumeWithException(it) }
            // ML Kit exposes no cancellation, so a cancelled coroutine simply stops
            // waiting; the task completes and its result is discarded.
            continuation.invokeOnCancellation { task.result }
        }

        val scale = pixelsPerPoint(bitmap.width, bitmap.height, pageSize)

        return buildList {
            for (block in recognised.textBlocks) {
                for (line in block.lines) {
                    val box = line.boundingBox ?: continue
                    val text = line.text
                    if (text.isBlank()) continue
                    add(segmentFor(box.left, box.top, box.right, box.bottom, text, scale))
                }
            }
        }
    }

    companion object {
        /**
         * Pixels the long edge is rendered at for recognition.
         *
         * Recognition accuracy is governed by glyph height in pixels, not by the
         * page's physical size, and this document's pages are unusually large —
         * 1647 x 901 pt, roughly 23 inches wide. Fixing the *pixel* budget rather
         * than a DPI keeps a wide page and a small one both landing somewhere the
         * model can read, and keeps the bitmap bounded either way.
         */
        const val TARGET_LONG_EDGE_PX = 3000f

        /**
         * Render scale to recognise [pageSize] at.
         *
         * Never below 1.0: rendering a page smaller than its natural size to read
         * it is self-defeating. Bounded above by [RenderScale.MAX_PIXELS], the same
         * ceiling every other render obeys, so recognition cannot be the one path
         * that fails in `Bitmap.createBitmap`.
         */
        fun scaleFor(pageSize: PageSize): Float {
            if (pageSize.widthPoints <= 0f || pageSize.heightPoints <= 0f) return 1f
            val longEdge = maxOf(pageSize.widthPoints, pageSize.heightPoints)
            val ideal = TARGET_LONG_EDGE_PX / longEdge

            val maxScale = kotlin.math.sqrt(
                RenderScale.MAX_PIXELS /
                    (pageSize.widthPoints.toDouble() * pageSize.heightPoints.toDouble()),
            ).toFloat()

            return min(ideal.coerceAtLeast(1f), maxScale.coerceAtLeast(0.25f))
        }

        /**
         * Pixels per point, taken from the bitmap actually produced.
         *
         * Derived from the rendered bitmap rather than the scale that was
         * requested, because the two can differ: the renderer rounds, and it caps
         * very large pages. Using the requested scale would place every recognised
         * run slightly off on exactly those pages, and the error would grow with
         * distance down the page — the kind of drift that looks like a selection
         * bug rather than a scaling one.
         *
         * The two axes are averaged; they agree to within a rounded pixel.
         */
        fun pixelsPerPoint(pixelWidth: Int, pixelHeight: Int, pageSize: PageSize): Float {
            if (pageSize.widthPoints <= 0f || pageSize.heightPoints <= 0f) return 1f
            val horizontal = pixelWidth / pageSize.widthPoints
            val vertical = pixelHeight / pageSize.heightPoints
            return (horizontal + vertical) / 2f
        }

        /**
         * One recognised line, in page points from the top-left.
         *
         * ML Kit reports pixels from the bitmap's top-left, and [TextSegment] is
         * already top-left with y downwards, so this is a divide and no flip. The
         * flip the engine's own extraction performs is for PDF's bottom-left
         * origin, which a rendered bitmap does not have — applying it here too
         * would put every recognised run on the wrong half of the page.
         *
         * Takes plain pixel bounds rather than an `android.graphics.Rect` so it can
         * be tested off-device. The stubbed `Rect` in the unit-test `android.jar`
         * has a no-op constructor and reads back as all zeroes, which does not fail
         * loudly — it quietly makes every assertion about position compare 0 to 0.
         */
        fun segmentFor(
            left: Int,
            top: Int,
            right: Int,
            bottom: Int,
            text: String,
            pixelsPerPoint: Float,
        ): TextSegment {
            // A zero scale would send every coordinate to infinity, and every
            // highlight would vanish rather than land in the wrong place — much
            // harder to recognise from a bug report.
            val scale = if (pixelsPerPoint > 0f && pixelsPerPoint.isFinite()) {
                pixelsPerPoint
            } else {
                1f
            }
            return TextSegment(
                left = left / scale,
                top = top / scale,
                right = right / scale,
                bottom = bottom / scale,
                text = text,
            )
        }
    }
}
