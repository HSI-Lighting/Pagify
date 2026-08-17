package com.hsilighting.pagify.ui.components

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.PageSize

/**
 * The visible region of the current page, in page-relative fractions (0..1).
 *
 * Fractions rather than pixels so the navigator never needs to know about zoom
 * levels, densities or scroll ranges — the caller resolves all of that once.
 */
data class ViewportWindow(
    val left: Float,
    val top: Float,
    val width: Float,
    val height: Float,
) {
    /** True when the whole page is visible, i.e. there is nothing to navigate. */
    val coversEverything: Boolean get() = width >= 0.999f && height >= 0.999f

    companion object {
        val Full = ViewportWindow(0f, 0f, 1f, 1f)
    }
}

/**
 * A minimap of the current page showing which part of it is on screen.
 *
 * Appears only while zoomed in, because at fit-width it would always show a
 * full-page rectangle and earn nothing. Tapping or dragging inside it recentres
 * the viewport, which is far quicker than repeatedly panning at high zoom.
 */
@Composable
fun PageNavigator(
    pageIndex: Int,
    pageSize: PageSize?,
    window: ViewportWindow,
    onRecenter: (fractionX: Float, fractionY: Float) -> Unit,
    thumbnailRenderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
    width: Dp = NAVIGATOR_WIDTH,
) {
    val aspect = pageSize?.aspectRatio ?: 0.7f

    // Rendered once per page at a fixed small size. It goes through the same
    // native cache as everything else, under its own zoom key, so revisiting a
    // page costs a memcpy rather than a re-render.
    val thumbnail: State<Bitmap?> = produceState<Bitmap?>(null, pageIndex, pageSize) {
        val size = pageSize ?: return@produceState
        if (size.widthPoints <= 0f) return@produceState
        val scale = (THUMBNAIL_WIDTH_PX / size.widthPoints).coerceAtLeast(0.25f)
        value = thumbnailRenderer(pageIndex, scale)
    }

    val recenter = rememberUpdatedState(onRecenter)
    var boxSize by remember { mutableStateOf(Size.Zero) }

    /** Map a touch inside the minimap to the page fraction it should centre on. */
    fun emit(position: Offset) {
        if (boxSize.width <= 0f || boxSize.height <= 0f) return
        recenter.value(
            (position.x / boxSize.width).coerceIn(0f, 1f),
            (position.y / boxSize.height).coerceIn(0f, 1f),
        )
    }

    Box(
        modifier = modifier
            .padding(8.dp)
            .width(width)
            .aspectRatio(aspect)
            .clip(RoundedCornerShape(6.dp))
            .background(Color.White.copy(alpha = 0.92f))
            .pointerInput(Unit) {
                boxSize = Size(size.width.toFloat(), size.height.toFloat())
                detectTapGestures { emit(it) }
            }
            .pointerInput(Unit) {
                boxSize = Size(size.width.toFloat(), size.height.toFloat())
                detectDragGestures { change, _ -> emit(change.position) }
            }
            // drawWithContent, so the indicator paints over the thumbnail without
            // needing a second layout pass or a Canvas sibling.
            .drawWithContent {
                drawContent()
                val rect = Rect(
                    left = window.left * size.width,
                    top = window.top * size.height,
                    right = (window.left + window.width) * size.width,
                    bottom = (window.top + window.height) * size.height,
                )
                // Dim what is off screen as four bands around the viewport, rather
                // than a full-cover rect punched through with BlendMode.Clear —
                // Clear needs its own graphics layer to composite correctly, and
                // without one it would paint solid black over the thumbnail.
                val dim = Color.Black.copy(alpha = 0.3f)
                drawRect(dim, size = Size(size.width, rect.top))
                drawRect(
                    dim,
                    topLeft = Offset(0f, rect.bottom),
                    size = Size(size.width, size.height - rect.bottom),
                )
                drawRect(
                    dim,
                    topLeft = Offset(0f, rect.top),
                    size = Size(rect.left, rect.height),
                )
                drawRect(
                    dim,
                    topLeft = Offset(rect.right, rect.top),
                    size = Size(size.width - rect.right, rect.height),
                )
                drawRect(
                    color = INDICATOR_COLOR,
                    topLeft = rect.topLeft,
                    size = rect.size,
                    style = Stroke(width = 2.dp.toPx()),
                )
            },
    ) {
        thumbnail.value?.let { bmp ->
            Image(
                bitmap = bmp.asImageBitmap(),
                contentDescription = "Page ${pageIndex + 1} navigator",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Fit,
            )
        }
    }
}

private val NAVIGATOR_WIDTH = 88.dp

/** Fixed thumbnail width in pixels; small enough that rendering it is negligible. */
private const val THUMBNAIL_WIDTH_PX = 180f

private val INDICATOR_COLOR = Color(0xFF3F5F90)
