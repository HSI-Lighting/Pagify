package com.hsilighting.pagify.ui.reader

import android.graphics.Bitmap
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
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.ui.components.PdfPageView
import com.hsilighting.pagify.ui.components.ViewportWindow
import com.hsilighting.pagify.ui.components.pinchToZoom
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest

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
    onWindowChanged: (ViewportWindow) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    /** Where to centre when the view opens, as a 0..1 fraction of the page. */
    initialFocus: Offset?,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier.fillMaxSize().clipToBounds()) {
        val density = LocalDensity.current
        val viewportW = with(density) { maxWidth.toPx() }
        val viewportH = with(density) { maxHeight.toPx() }

        val aspect = pageSize?.aspectRatio ?: DEFAULT_ASPECT
        // The page at 1.0 fills the viewport width; everything scales from there.
        val baseW = viewportW
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

        Box(
            Modifier
                .fillMaxSize()
                .pinchToZoom { factor, centroid -> zoomAbout(factor, centroid) }
                .pointerInput(Unit) {
                    detectDragGestures { change, drag ->
                        change.consume()
                        offset = clamp(offset + drag, scale)
                    }
                }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onDoubleTap = { position ->
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
                },
        ) {
            val residual = if (committedScale > 0f) scale / committedScale else 1f

            PdfPageView(
                pageIndex = pageIndex,
                // Laid out at the committed scale; `residual` covers the gap while
                // a gesture is still in flight, so the visual is always correct
                // even before the page has been re-rasterised.
                pageWidth = with(density) { (baseW * committedScale).toDp() },
                pageSizeProvider = pageSizeProvider,
                renderer = renderer,
                modifier = Modifier.graphicsLayer {
                    transformOrigin = TransformOrigin(0f, 0f)
                    scaleX = residual
                    scaleY = residual
                    translationX = offset.x
                    translationY = offset.y
                },
            )
        }
    }
}

private const val DEFAULT_ASPECT = 595f / 842f
private const val DOUBLE_TAP_ZOOM = 2.5f

/** How long after the last gesture event to re-rasterise at the new scale. */
private const val SETTLE_MILLIS = 180L
