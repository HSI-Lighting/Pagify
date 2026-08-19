package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Rect

/**
 * Capturing a region of a page as a picture.
 *
 * **Not a screenshot.** The region is re-rendered from the document by the engine,
 * which is what makes "only the contents of the PDF — no notifications, no popups
 * from other apps" a property of the design rather than something to filter out:
 * those pixels never exist. Roadmap decision 4.8 records the alternatives
 * (`MediaProjection`, `PixelCopy`) and why each was rejected.
 *
 * Everything here is pure so it can be tested off-device; the file writing,
 * sharing and gallery insertion live in `CaptureExport`.
 */
enum class CaptureFormat(
    /** What the engine's `captureRegion` expects. */
    val wireName: String,
    val extension: String,
    val mimeType: String,
) {
    /**
     * Lossless, and the right default: a page is line art and type, where PNG is
     * both exact and small.
     */
    PNG("png", "png", "image/png"),

    /**
     * For the other case. A scanned page is a photograph, and a lossless encode of
     * a photograph is enormous — the scanned catalogue is exactly that, which is
     * why the choice is offered rather than decided.
     */
    JPEG("jpeg", "jpg", "image/jpeg"),
}

/**
 * How sharp a capture to take, as a multiple of the page's natural size.
 *
 * Independent of the on-screen zoom on purpose. The display can only show so many
 * pixels; a capture is kept, zoomed into and printed, so it is worth more than the
 * screen can render. [X2] is the default — visibly sharper than the display on
 * every device we target, and still a file small enough to send.
 */
enum class CaptureScale(val factor: Float, val label: String) {
    X1(1f, "1×"),
    X2(2f, "2×"),
    X4(4f, "4×"),
}

/** One capture, fully specified. The crop is in page points, top-left origin. */
data class CaptureRequest(
    val pageIndex: Int,
    val crop: Rect,
    val scale: CaptureScale = CaptureScale.X2,
    val format: CaptureFormat = CaptureFormat.PNG,
    /** 1–100. Ignored for PNG. */
    val quality: Int = 92,
)

/**
 * The smallest crop worth capturing, in page points.
 *
 * Below this a drag is a tap that moved: a stray finger on the page should not
 * produce a two-pixel image and a share sheet. Roughly 3 mm square on paper.
 */
const val MINIMUM_CAPTURE_POINTS = 8f

/** True when a dragged rectangle is big enough to mean it. */
fun Rect.isWorthCapturing(): Boolean =
    width >= MINIMUM_CAPTURE_POINTS && height >= MINIMUM_CAPTURE_POINTS

/**
 * A rectangle from two corners, in whichever order they were dragged.
 *
 * `Rect(topLeft, bottomRight)` does not normalise, and a drag up-and-left
 * produces a rectangle with negative width that silently captures nothing.
 */
fun rectFromCorners(startX: Float, startY: Float, endX: Float, endY: Float): Rect = Rect(
    left = minOf(startX, endX),
    top = minOf(startY, endY),
    right = maxOf(startX, endX),
    bottom = maxOf(startY, endY),
)

/**
 * File name for a capture.
 *
 * Sortable, unambiguous and safe on every filesystem: the source document, the
 * page as the reader numbers it, and the time. A gallery full of `image_1.png`
 * is what this exists to avoid.
 */
fun captureFileName(
    documentName: String,
    pageIndex: Int,
    format: CaptureFormat,
    timestamp: String,
): String {
    val stem = documentName
        .substringBeforeLast('.')
        .replace(Regex("[^A-Za-z0-9 _-]"), "")
        .trim()
        .take(48)
        .ifEmpty { "Pagify" }
    return "$stem p${pageIndex + 1} $timestamp.${format.extension}"
}
