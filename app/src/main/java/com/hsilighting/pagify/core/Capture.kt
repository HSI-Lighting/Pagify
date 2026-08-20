package com.hsilighting.pagify.core

import androidx.compose.ui.geometry.Offset
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

/**
 * One page's share of a capture.
 *
 * [crop] is in that page's own points; [dest] is where it belongs in the picture,
 * in capture units. The two are separate because a capture is not a page: it is a
 * rectangle someone dragged on screen, which may take the bottom of one page, the
 * gap below it, and the top of the next.
 */
data class CaptureTile(
    val pageIndex: Int,
    val crop: Rect,
    val dest: Rect,
)

/**
 * One capture, fully specified.
 *
 * Sized in **capture units** — screen pixels at the zoom the reader was at — with
 * [scale] multiplying that for the export. The framing comes from what was on
 * screen; the resolution does not, which is what lets a capture be sharper than
 * the display that framed it.
 */
data class CaptureRequest(
    val tiles: List<CaptureTile>,
    val width: Float,
    val height: Float,
    /**
     * Shows wherever no page reaches: between two pages, and past the edge of one.
     * The reader's own background, so a capture looks like what was on screen.
     */
    val background: Long,
    /**
     * The page the drag started on.
     *
     * For the file name and the label on the sheet only. A capture spanning two
     * pages has to be called something, and the page someone started on is the one
     * they would name it after.
     */
    val originPage: Int,
    val scale: CaptureScale = CaptureScale.X2,
    val format: CaptureFormat = CaptureFormat.PNG,
    /** 1–100. Ignored for PNG. */
    val quality: Int = 92,
    /**
     * The ring drawn with the lasso, in capture units; empty for a plain box.
     *
     * The picture is still the ring's bounding box — an image is a rectangle — but
     * everything outside the ring is painted over with [background] by the engine,
     * so a detail can be lifted off a busy drawing without the things beside it.
     *
     * Part of the request rather than a one-off argument because the editor
     * re-exports at other scales and formats, and a mask that survived only the
     * first render would quietly come back as a rectangle the moment someone
     * chose 4×.
     */
    val mask: List<Offset> = emptyList(),
) {
    /**
     * The picture's own coordinate space, which is what markup is drawn in.
     *
     * A mark drawn across the join between two pages belongs to neither of them,
     * so marks are positioned against the capture rather than against any page
     * inside it.
     */
    val localBounds: Rect get() = Rect(0f, 0f, width, height)
}

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

/**
 * What fills a capture where no page reaches.
 *
 * Two places show it: the gap between two pages, and — the reason this is a choice
 * at all — everything outside a ring drawn with the lasso. A detail lifted off a
 * drawing is usually going somewhere else, and where it is going decides what
 * should be behind it: white for a document, black for a dark slide, nothing at
 * all for dropping it onto a background that already exists.
 *
 * [PAGE] is the default and follows the reader's own backdrop, so a capture looks
 * like what was on screen. Its colour is null because only the reader knows it —
 * it comes from the theme, and the theme can change between one capture and the
 * next.
 */
enum class CaptureFill(val label: String, val colour: Long?) {
    PAGE("Page", null),
    WHITE("White", 0xFFFFFFFFL),
    BLACK("Black", 0xFF000000L),

    /**
     * No fill at all: those pixels are cut out of the picture.
     *
     * Only PNG can carry this, so choosing it moves the export to PNG — see
     * `setCaptureFill`. The engine reads any alpha short of opaque as a cut-out
     * rather than a veil, because a half-transparent fill would leave the page
     * faintly readable outside the ring, which is what the ring was drawn to
     * prevent.
     */
    TRANSPARENT("Transparent", 0x00000000L),
}
