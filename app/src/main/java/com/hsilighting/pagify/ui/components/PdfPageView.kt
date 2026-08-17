package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import android.graphics.Bitmap
import com.hsilighting.pagify.core.PageSize

/**
 * One rendered page.
 *
 * Renders lazily and asynchronously: the composable first claims its space using
 * the page's aspect ratio, then fills in pixels when they arrive. Sizing the
 * placeholder correctly up front is what stops the scroll position from jumping
 * as pages resolve.
 */
@Composable
fun PdfPageView(
    pageIndex: Int,
    zoom: Float,
    containerWidth: Dp,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
) {
    var pageSize by remember(pageIndex) { mutableStateOf<PageSize?>(null) }
    var bitmap by remember(pageIndex) { mutableStateOf<Bitmap?>(null) }
    var failed by remember(pageIndex) { mutableStateOf(false) }

    LaunchedEffect(pageIndex) { pageSize = pageSizeProvider(pageIndex) }

    val density = LocalDensity.current

    // The render scale converts PDF points to *device pixels* at the width this
    // page is being drawn at, so text is sharp rather than upscaled. Quantised to
    // 0.25 steps to match the native cache's own quantisation — without this,
    // every pixel of zoom drift would be a cache miss.
    val renderScale = remember(pageSize, zoom, containerWidth, density) {
        val size = pageSize ?: return@remember null
        if (size.widthPoints <= 0f) return@remember null
        val targetPx = with(density) { containerWidth.toPx() } * zoom
        val raw = targetPx / size.widthPoints
        (Math.ceil((raw / CACHE_ZOOM_QUANTUM).toDouble()) * CACHE_ZOOM_QUANTUM)
            .toFloat()
            .coerceAtLeast(CACHE_ZOOM_QUANTUM)
    }

    LaunchedEffect(pageIndex, renderScale) {
        val scale = renderScale ?: return@LaunchedEffect
        failed = false
        val rendered = renderer(pageIndex, scale)
        if (rendered == null) failed = true else bitmap = rendered
    }

    val aspect = pageSize?.aspectRatio ?: DEFAULT_ASPECT_RATIO

    Box(
        modifier = modifier
            .fillMaxWidth()
            .aspectRatio(aspect)
            .background(MaterialTheme.colorScheme.surfaceContainerHighest),
        contentAlignment = Alignment.Center,
    ) {
        val current = bitmap
        when {
            current != null -> Image(
                bitmap = current.asImageBitmap(),
                contentDescription = "Page ${pageIndex + 1}",
                modifier = Modifier.fillMaxWidth(),
                contentScale = ContentScale.FillWidth,
            )
            // A failed page keeps its placeholder rather than collapsing, so the
            // rest of the document stays scrollable.
            failed -> Box(Modifier)
            else -> CircularProgressIndicator(strokeWidth = 2.dp)
        }
    }
}

/** Must match `ZOOM_QUANTUM` in `rust/pdf_core/src/render/cache.rs`. */
private const val CACHE_ZOOM_QUANTUM = 0.25f

/** A4 portrait — a reasonable guess before a page has been measured. */
private const val DEFAULT_ASPECT_RATIO = 595f / 842f
