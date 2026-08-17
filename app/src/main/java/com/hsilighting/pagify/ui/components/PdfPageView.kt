package com.hsilighting.pagify.ui.components

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
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
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.RenderScale

/**
 * One rendered page, drawn at exactly [pageWidth].
 *
 * [pageWidth] is the *drawn* width, not the viewport width — when zoomed it
 * exceeds the screen and the surrounding horizontal scroll pans across it. The
 * render resolution is derived from the same number, so magnifying a page
 * re-rasterises it sharp instead of upscaling a stale bitmap.
 *
 * The placeholder claims its space from the page's real aspect ratio as soon as
 * that is known, which is what stops the scroll position from lurching as pages
 * resolve.
 */
@Composable
fun PdfPageView(
    pageIndex: Int,
    pageWidth: Dp,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
) {
    var pageSize by remember(pageIndex) { mutableStateOf<PageSize?>(null) }
    var bitmap by remember(pageIndex) { mutableStateOf<Bitmap?>(null) }

    LaunchedEffect(pageIndex) { pageSize = pageSizeProvider(pageIndex) }

    val density = LocalDensity.current

    // Points -> device pixels at the width this page is actually drawn at, so text
    // is rasterised sharp rather than upscaled. Quantised to 0.25 steps to match
    // the native cache's own quantisation; without that, every pixel of zoom drift
    // would be a cache miss.
    // Shared with the prefetcher via RenderScale, so the cache is warmed under the
    // exact key this view will ask for.
    val renderScale = remember(pageSize, pageWidth, density) {
        val size = pageSize ?: return@remember null
        RenderScale.forPage(size, with(density) { pageWidth.toPx() })
    }

    LaunchedEffect(pageIndex, renderScale) {
        val scale = renderScale ?: return@LaunchedEffect
        renderer(pageIndex, scale)?.let { bitmap = it }
    }

    val aspect = pageSize?.aspectRatio ?: DEFAULT_ASPECT_RATIO

    Box(
        modifier = modifier
            .width(pageWidth)
            .aspectRatio(aspect)
            .background(MaterialTheme.colorScheme.surfaceContainerHighest),
        contentAlignment = Alignment.Center,
    ) {
        val current = bitmap
        if (current != null) {
            Image(
                bitmap = current.asImageBitmap(),
                contentDescription = "Page ${pageIndex + 1}",
                modifier = Modifier.fillMaxWidth(),
                contentScale = ContentScale.FillWidth,
            )
        } else {
            // A page that failed keeps its placeholder rather than collapsing, so
            // the rest of the document stays scrollable.
            CircularProgressIndicator(strokeWidth = 2.dp)
        }
    }
}

/** A4 portrait — a guess used only until a page has been measured. */
private const val DEFAULT_ASPECT_RATIO = 595f / 842f
