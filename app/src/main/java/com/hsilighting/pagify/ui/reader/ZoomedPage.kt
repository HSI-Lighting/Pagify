package com.hsilighting.pagify.ui.reader

import android.graphics.Bitmap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PenMode
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.core.RenderScale
import com.hsilighting.pagify.ui.components.ViewportWindow
import com.hsilighting.pagify.ui.components.annotationLayer
import com.hsilighting.pagify.ui.components.pinchToZoom
import com.hsilighting.pagify.ui.components.twoFingerPanXY
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlin.math.roundToInt

/**
 * A single page, magnified.
 *
 * Zooming deliberately leaves the continuous list behind and scopes the view to
 * one page: panning a magnified page should never wander into its neighbours,
 * which is disorienting and loses your place.
 *
 * ## Why this does its own transform instead of using scroll containers
 *
 * The position of the content is held here as an explicit [Offset], and applied
 * with `graphicsLayer`. That is what makes pinch anchoring *exact*: the new
 * offset is computed and applied in the same frame as the gesture event.
 *
 * The earlier attempt drove layout width from zoom and let two `ScrollState`s
 * position it. Anchoring then had to be deferred, because a scroll range does not
 * exist until the page has been re-measured — and a pinch fires dozens of events,
 * each restarting and cancelling the pending correction. The surviving one
 * applied a stale ratio against a scroll offset that had already moved, so the
 * zoom drifted away from the fingers. Nothing here waits for a frame.
 *
 * Sharpness is preserved separately: the page is *laid out* at `committedScale`,
 * and once a gesture settles that catches up to the live scale, so the page is
 * re-rasterised rather than left as an upscaled bitmap.
 */
@Composable
fun ZoomedPage(
    pageIndex: Int,
    initialZoom: Float,
    pageSize: PageSize?,
    /** Called when a gesture settles, with the scale to render and prefetch at. */
    onZoomSettled: (Float) -> Unit,
    /** Fired on every zoom gesture event, to drive the blank-frame watcher. */
    onZoomActivity: () -> Unit,
    onWindowChanged: (ViewportWindow) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    /** Whatever the list last drew for this page; avoids a blank first frame. */
    initialBitmap: Bitmap?,
    /**
     * The width, in pixels, that the list draws this page at.
     *
     * This is what `scale = 1.0` has to mean, and getting it wrong is visible as a
     * jump. This view replaces the whole row including the thumbnail rail, so its
     * own viewport is *wider* than the reader area the page was just occupying —
     * measuring the base against the viewport made entering zoom silently enlarge
     * the page by the width of the rail and its gaps before any gesture applied.
     */
    basePageWidthPx: Float,
    /** Where to centre when the view opens, as a 0..1 fraction of the page. */
    initialFocus: Offset?,
    /**
     * Marks already on this page, and the state of the tool ribbon.
     *
     * Annotating has to work here too. This view is a separate render path from
     * the list, and it was built before the tools existed, so it drew the page
     * bitmap and nothing else: magnifying a page made its highlights disappear,
     * and the pen had no surface to draw on — every one-finger drag went to the
     * pan handler below instead.
     */
    annotations: List<Annotation>,
    textSegments: List<TextSegment>,
    tool: AnnotationTool,
    penMode: PenMode,
    penColor: Long,
    onAddAnnotation: (Annotation) -> Unit,
    /** The Note tool was tapped at this page point. */
    onRequestNote: (pageIndex: Int, anchor: Offset) -> Unit,
    onEraseStart: () -> Unit,
    onErase: (point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier.fillMaxSize().clipToBounds()) {
        val density = LocalDensity.current
        val viewportW = with(density) { maxWidth.toPx() }
        val viewportH = with(density) { maxHeight.toPx() }

        val aspect = pageSize?.aspectRatio ?: DEFAULT_ASPECT
        // Scale 1.0 is exactly what the list was showing, so handing over is
        // pixel-identical and every zoom grows continuously from there.
        val baseW = if (basePageWidthPx > 0f) basePageWidthPx else viewportW
        val baseH = if (aspect > 0f) baseW / aspect else viewportH

        var scale by remember { mutableFloatStateOf(initialZoom) }
        var committedScale by remember { mutableFloatStateOf(initialZoom) }
        var offset by remember { mutableStateOf(Offset.Zero) }

        /**
         * Keep the content covering the viewport, and centred on any axis where it
         * is smaller. Without this a pan could strand the page off screen.
         */
        fun clamp(candidate: Offset, atScale: Float): Offset {
            val contentW = baseW * atScale
            val contentH = baseH * atScale
            val x = if (contentW <= viewportW) {
                (viewportW - contentW) / 2f
            } else {
                candidate.x.coerceIn(viewportW - contentW, 0f)
            }
            val y = if (contentH <= viewportH) {
                (viewportH - contentH) / 2f
            } else {
                candidate.y.coerceIn(viewportH - contentH, 0f)
            }
            return Offset(x, y)
        }

        // Open centred on whatever the entering gesture was aimed at.
        LaunchedEffect(initialFocus, baseW, baseH) {
            val focus = initialFocus ?: return@LaunchedEffect
            offset = clamp(
                Offset(
                    viewportW / 2f - focus.x * baseW * scale,
                    viewportH / 2f - focus.y * baseH * scale,
                ),
                scale,
            )
        }

        /**
         * Scale about [focus], a point in viewport coordinates.
         *
         * The content point under the focus is `(focus - offset) / scale`. For it
         * to stay under the focus at the new scale, the offset must become
         * `focus - thatPoint * newScale`, which reduces to the expression below.
         */
        fun zoomAbout(factor: Float, focus: Offset) {
            val newScale = (scale * factor).coerceIn(
                PdfReaderState.MIN_ZOOM,
                PdfReaderState.MAX_ZOOM,
            )
            if (newScale == scale) return
            val ratio = newScale / scale
            offset = clamp(focus - (focus - offset) * ratio, newScale)
            scale = newScale
        }

        // Re-render at the settled scale, and let the rest of the app know. Held
        // back until the gesture stops so a pinch does not rasterise the page at
        // every intermediate size.
        LaunchedEffect(Unit) {
            snapshotFlow { scale }.collectLatest { settled ->
                delay(SETTLE_MILLIS)
                committedScale = settled
                onZoomSettled(settled)
            }
        }

        // The rasterised page. Kept here rather than in PdfPageView because the
        // zoomed view draws it itself.
        // Seeded synchronously from whatever the list last drew for this page, so
        // the very first composed frame already has pixels. Without this there is
        // a measurable window — 54 ms on this device — where the content area is
        // completely blank while the proxy render is still in flight.
        var pageBitmap by remember(pageIndex) { mutableStateOf(initialBitmap) }
        var bitmapScale by remember(pageIndex) { mutableStateOf(0f) }

        val renderScale = remember(pageSize, committedScale, baseW) {
            val size = pageSize ?: return@remember null
            RenderScale.forPage(size, baseW * committedScale)
        }

        // Something to show *immediately*, before the sharp render exists.
        //
        // Entering zoom composes this view fresh, with no bitmap, so the canvas
        // had nothing to draw and the page flashed blank — worst on exactly the
        // documents where the sharp render takes longest. The proxy is the same
        // cheap raster the list already drew, so it is almost always a cache hit
        // and appears at once; it is then replaced, never blanked.
        LaunchedEffect(pageIndex, pageSize) {
            val size = pageSize ?: return@LaunchedEffect
            if (pageBitmap != null) return@LaunchedEffect // already seeded or drawn
            val proxy = RenderScale.proxyFor(size, baseW)
            renderer(pageIndex, proxy)?.let {
                if (bitmapScale < proxy) {
                    pageBitmap = it
                    bitmapScale = proxy
                }
            }
        }

        LaunchedEffect(pageIndex, renderScale) {
            val target = renderScale ?: return@LaunchedEffect
            if (bitmapScale >= target) return@LaunchedEffect
            // Assigned only on success, so a failed or slow render leaves whatever
            // is on screen in place rather than clearing it.
            renderer(pageIndex, target)?.let {
                pageBitmap = it
                bitmapScale = target
            }
        }

        // Report the visible region for the navigator.
        LaunchedEffect(offset, scale, baseW, baseH, viewportW, viewportH) {
            val contentW = baseW * scale
            val contentH = baseH * scale
            onWindowChanged(
                if (contentW <= 0f || contentH <= 0f) {
                    ViewportWindow.Full
                } else {
                    val w = (viewportW / contentW).coerceIn(0f, 1f)
                    val h = (viewportH / contentH).coerceIn(0f, 1f)
                    ViewportWindow(
                        left = (-offset.x / contentW).coerceIn(0f, 1f - w),
                        top = (-offset.y / contentH).coerceIn(0f, 1f - h),
                        width = w,
                        height = h,
                    )
                },
            )
        }

        // Pixels per page point at the size the canvas is currently drawing, which
        // is the live scale rather than the committed one: the bitmap may still be
        // the previous rasterisation stretched to fit, and a mark has to sit on the
        // text as it appears now, not as it will appear once the render catches up.
        val annotationScale = pageSize
            ?.takeIf { it.widthPoints > 0f }
            ?.let { baseW * scale / it.widthPoints }
            ?: 0f

        // With a tool live one finger belongs to the tool, exactly as in the list.
        // Leaving the pan on one finger is what made the pen appear disabled here:
        // `detectDragGestures` consumed the drag before the drawing layer saw it.
        val toolActive = tool != AnnotationTool.None

        Box(
            Modifier
                .fillMaxSize()
                .pinchToZoom { factor, centroid ->
                    onZoomActivity()
                    zoomAbout(factor, centroid)
                }
                // Two fingers always pan, for the same reason as in the list: the
                // pinch handler claims every two-finger event, so nothing else
                // would ever receive one.
                .twoFingerPanXY { drag -> offset = clamp(offset + drag, scale) }
                // One finger pans only when it is not busy annotating.
                .then(
                    if (toolActive) {
                        Modifier
                    } else {
                        Modifier.pointerInput(Unit) {
                            detectDragGestures { change, drag ->
                                change.consume()
                                offset = clamp(offset + drag, scale)
                            }
                        }
                    },
                )
                .pointerInput(Unit) {
                    detectTapGestures(
                        onDoubleTap = { position ->
                            onZoomActivity()
                            // Back to fit-width, about the tapped point. Reported
                            // immediately rather than after the settle delay so the
                            // pinned page is released without a visible lag.
                            if (scale > PdfReaderState.FIT_WIDTH_ZOOM + 0.01f) {
                                zoomAbout(PdfReaderState.FIT_WIDTH_ZOOM / scale, position)
                            } else {
                                zoomAbout(DOUBLE_TAP_ZOOM / scale, position)
                            }
                            committedScale = scale
                            onZoomSettled(scale)
                        },
                    )
                }
                // Last in the chain, so it is the innermost input receiver and a
                // one-finger drag reaches the tool before anything else can claim
                // it — and the innermost draw, so marks land over the page.
                .annotationLayer(
                    pageIndex = pageIndex,
                    annotations = annotations,
                    textSegments = textSegments,
                    tool = tool,
                    penMode = penMode,
                    penColor = penColor,
                    renderScale = annotationScale,
                    // The page is drawn translated by `offset`, so the layer has to
                    // be told where its top-left corner actually is.
                    contentOffset = offset,
                    onAdd = onAddAnnotation,
                    onRequestNote = { anchor -> onRequestNote(pageIndex, anchor) },
                    onEraseStart = onEraseStart,
                    onErase = onErase,
                    onEraseEnd = onEraseEnd,
                ),
        ) {
            // The page is *drawn*, not laid out, at its magnified size. Laying out
            // a composable at `baseW * scale` meant a 4x zoom asked for a
            // ~6400x9100 px layer, past the GPU's maximum texture size, and the
            // layer silently failed — the page vanished the instant a gesture
            // settled and the layout caught up to the scale. Here the canvas is
            // always viewport-sized and only the destination rectangle grows, so
            // there is no size that can break it.
            Canvas(Modifier.fillMaxSize()) {
                val bmp = pageBitmap ?: return@Canvas
                drawImage(
                    image = bmp.asImageBitmap(),
                    dstOffset = IntOffset(offset.x.roundToInt(), offset.y.roundToInt()),
                    dstSize = IntSize(
                        (baseW * scale).roundToInt().coerceAtLeast(1),
                        (baseH * scale).roundToInt().coerceAtLeast(1),
                    ),
                    // Bilinear, so the stretch between a settle and the next
                    // re-rasterisation is smooth rather than blocky.
                    filterQuality = FilterQuality.Medium,
                )
            }
        }
    }
}

private const val DEFAULT_ASPECT = 595f / 842f
private const val DOUBLE_TAP_ZOOM = 2.5f

/** How long after the last gesture event to re-rasterise at the new scale. */
private const val SETTLE_MILLIS = 180L
