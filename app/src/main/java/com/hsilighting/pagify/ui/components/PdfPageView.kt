package com.hsilighting.pagify.ui.components

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.RenderScale

/**
 * One page of the document, rendered in two passes.
 *
 * **Pass one is a proxy** at a fraction of the final size. It costs roughly a
 * sixteenth of a full render, so a page appears essentially immediately instead
 * of arriving blank and late while you are still scrolling past it.
 *
 * **Pass two is the readable render**, and it only happens once [readable] turns
 * true — i.e. when scrolling has settled and this is a page you have actually
 * landed on. Rasterising every page you flick past at reading resolution is pure
 * waste: at 50-99 ms each it is what makes fast scrolling feel heavy.
 *
 * The previous bitmap stays on screen until its replacement arrives, so a page
 * never blanks while being upgraded.
 */
@Composable
fun PdfPageView(
    pageIndex: Int,
    pageWidth: Dp,
    readable: Boolean,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
) {
    var pageSize by remember(pageIndex) { mutableStateOf<PageSize?>(null) }
    var bitmap by remember(pageIndex) { mutableStateOf<Bitmap?>(null) }
    /** Scale of what is currently on screen, so an upgrade is not redone. */
    var renderedScale by remember(pageIndex) { mutableStateOf(0f) }

    LaunchedEffect(pageIndex) { pageSize = pageSizeProvider(pageIndex) }

    val density = LocalDensity.current
    val targetPx = with(density) { pageWidth.toPx() }

    val proxyScale = remember(pageSize, targetPx) {
        pageSize?.let { RenderScale.proxyFor(it, targetPx) }
    }
    val readableScale = remember(pageSize, targetPx) {
        pageSize?.let { RenderScale.forPage(it, targetPx) }
    }

    // Proxy first, unconditionally: cheap, and it is what fills the placeholder.
    LaunchedEffect(pageIndex, proxyScale) {
        val scale = proxyScale ?: return@LaunchedEffect
        if (renderedScale >= scale) return@LaunchedEffect
        renderer(pageIndex, scale)?.let {
            if (renderedScale < scale) {
                bitmap = it
                renderedScale = scale
            }
        }
    }

    // Then the readable pass, only where it earns its cost.
    LaunchedEffect(pageIndex, readableScale, readable) {
        if (!readable) return@LaunchedEffect
        val scale = readableScale ?: return@LaunchedEffect
        if (renderedScale >= scale) return@LaunchedEffect
        renderer(pageIndex, scale)?.let {
            bitmap = it
            renderedScale = scale
        }
    }

    val aspect = pageSize?.aspectRatio ?: DEFAULT_ASPECT_RATIO

    Box(
        modifier = modifier
            .width(pageWidth)
            .aspectRatio(aspect)
            .background(MaterialTheme.colorScheme.surfaceContainerHighest),
        contentAlignment = Alignment.Center,
    ) {
        bitmap?.let { bmp ->
            Image(
                bitmap = bmp.asImageBitmap(),
                contentDescription = "Page ${pageIndex + 1}",
                modifier = Modifier.fillMaxWidth(),
                contentScale = ContentScale.FillWidth,
                // Bilinear, so a proxy reads as soft rather than blocky while the
                // readable pass is still on its way.
                filterQuality = FilterQuality.Medium,
            )
        }
    }
}

/** A4 portrait — a guess used only until a page has been measured. */
private const val DEFAULT_ASPECT_RATIO = 595f / 842f
