package com.hsilighting.pagify.ui.reader

import android.graphics.Bitmap
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.ui.components.PdfPageView
import com.hsilighting.pagify.ui.components.ViewportWindow
import com.hsilighting.pagify.ui.components.pinchToZoom
import kotlin.math.roundToInt

/**
 * Where the viewport should sit after a zoom change, expressed as the state
 * *before* it happened.
 *
 * Captured at gesture time rather than applied immediately because the new scroll
 * bounds do not exist until the page has been re-measured at its new size.
 */
private data class ZoomAnchor(
    val previousZoom: Float,
    /** Focal point in viewport coordinates: the midpoint between the fingers. */
    val focus: Offset,
    val scrollX: Int,
    val scrollY: Int,
)

/**
 * A single page, magnified.
 *
 * Zooming deliberately leaves the continuous list behind and scopes the view to
 * one page: panning a magnified page should never wander into its neighbours,
 * which is disorienting and loses your place. Both axes scroll here, and both are
 * bounded by the page itself.
 */
@Composable
fun ZoomedPage(
    pageIndex: Int,
    zoom: Float,
    pageSize: PageSize?,
    onZoomBy: (Float) -> Unit,
    onToggleZoom: () -> Unit,
    onWindowChanged: (ViewportWindow) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    recenterRequest: Offset?,
    onRecenterHandled: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val horizontal = rememberScrollState()
    val vertical = rememberScrollState()
    val density = LocalDensity.current

    var anchor by remember { mutableStateOf<ZoomAnchor?>(null) }

    BoxWithConstraints(modifier.fillMaxSize()) {
        val viewportWidth: Dp = maxWidth
        val viewportHeight: Dp = maxHeight
        val pageWidth = viewportWidth * zoom

        val viewportWidthPx = with(density) { viewportWidth.toPx() }
        val viewportHeightPx = with(density) { viewportHeight.toPx() }
        val contentWidthPx = with(density) { pageWidth.toPx() }
        val aspect = pageSize?.aspectRatio ?: DEFAULT_ASPECT
        val contentHeightPx = if (aspect > 0f) contentWidthPx / aspect else 0f

        // ------------------------------------------------- centroid anchoring --
        // Keep whatever was under the fingers under the fingers. Working in
        // viewport coordinates: the content point at the focus is
        // (scroll + focus); after scaling by `ratio` it sits at
        // (scroll + focus) * ratio, so the scroll must absorb the difference.
        LaunchedEffect(zoom) {
            val pending = anchor ?: return@LaunchedEffect
            anchor = null
            if (pending.previousZoom <= 0f) return@LaunchedEffect

            // The new scroll range only exists once the page has been re-measured,
            // and `scrollTo` clamps to the current maximum — so applying this in
            // the same frame would silently clamp against the old, smaller bounds.
            withFrameNanos { }
            withFrameNanos { }

            val ratio = zoom / pending.previousZoom
            val targetX = (pending.scrollX + pending.focus.x) * ratio - pending.focus.x
            val targetY = (pending.scrollY + pending.focus.y) * ratio - pending.focus.y

            horizontal.scrollTo(targetX.roundToInt().coerceIn(0, horizontal.maxValue))
            vertical.scrollTo(targetY.roundToInt().coerceIn(0, vertical.maxValue))
        }

        // Recentre requests from the navigator, expressed as page fractions.
        LaunchedEffect(recenterRequest) {
            val request = recenterRequest ?: return@LaunchedEffect
            // Same reason as the anchoring effect: the scroll range does not exist
            // until the page has been measured at its new size, and `scrollTo`
            // clamps against whatever the maximum currently is.
            withFrameNanos { }
            withFrameNanos { }
            val targetX = request.x * contentWidthPx - viewportWidthPx / 2f
            val targetY = request.y * contentHeightPx - viewportHeightPx / 2f
            horizontal.scrollTo(targetX.roundToInt().coerceIn(0, horizontal.maxValue))
            vertical.scrollTo(targetY.roundToInt().coerceIn(0, vertical.maxValue))
            onRecenterHandled()
        }

        // Report the visible region so the navigator can draw it. Reading the
        // scroll values here is what makes this recompute while panning.
        val window = remember(
            horizontal.value, vertical.value,
            contentWidthPx, contentHeightPx, viewportWidthPx, viewportHeightPx,
        ) {
            if (contentWidthPx <= 0f || contentHeightPx <= 0f) {
                ViewportWindow.Full
            } else {
                val widthFraction = (viewportWidthPx / contentWidthPx).coerceIn(0f, 1f)
                val heightFraction = (viewportHeightPx / contentHeightPx).coerceIn(0f, 1f)
                ViewportWindow(
                    left = (horizontal.value / contentWidthPx).coerceIn(0f, 1f - widthFraction),
                    top = (vertical.value / contentHeightPx).coerceIn(0f, 1f - heightFraction),
                    width = widthFraction,
                    height = heightFraction,
                )
            }
        }
        LaunchedEffect(window) { onWindowChanged(window) }

        // Two nested boxes on purpose. The gesture detectors sit on the OUTER,
        // viewport-sized box so the focal point they report is in viewport
        // coordinates — the space the anchoring maths above is written in. Placed
        // inside the scrollers instead, their coordinates would be content-space
        // (already shifted by the scroll offset) and every zoom would land in the
        // wrong place by exactly the current scroll.
        Box(
            Modifier
                .fillMaxSize()
                .pinchToZoom { factor, centroid ->
                    anchor = ZoomAnchor(zoom, centroid, horizontal.value, vertical.value)
                    onZoomBy(factor)
                }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onDoubleTap = { position ->
                            // Same anchoring for double-tap: zoom about the point
                            // tapped, not about a corner.
                            anchor = ZoomAnchor(zoom, position, horizontal.value, vertical.value)
                            onToggleZoom()
                        },
                    )
                },
        ) {
            Box(
                Modifier
                    .fillMaxSize()
                    .horizontalScroll(horizontal)
                    .verticalScroll(vertical),
            ) {
                PdfPageView(
                    pageIndex = pageIndex,
                    pageWidth = pageWidth,
                    pageSizeProvider = pageSizeProvider,
                    renderer = renderer,
                )
            }
        }
    }
}

private const val DEFAULT_ASPECT = 595f / 842f
