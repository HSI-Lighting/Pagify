package com.hsilighting.pagify.ui.components

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.DragInteraction
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.RenderScale

/**
 * A scrollable rail of page thumbnails.
 *
 * This exists because browsing and reading are different jobs with wildly
 * different costs. Finding a page only needs it to be *recognisable*: a thumbnail
 * is around 0.03 MP against 2.3 MP for a readable page, so the whole document can
 * be flicked through for roughly the price of one full render, and every
 * thumbnail of a 149-page document fits in about 19 MB.
 *
 * Tapping a thumbnail jumps the reader there, at which point that one page — and
 * only that page — is rendered at full resolution.
 */
@Composable
fun ThumbnailStrip(
    pageCount: Int,
    currentPage: Int,
    onSelectPage: (Int) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
) {
    val listState = rememberLazyListState()

    /**
     * Set once you drag the rail yourself, cleared when the reader moves.
     *
     * Browsing the rail ahead of the page you are reading is the whole point of
     * having one, so the follow-the-reader behaviour below has to yield the
     * moment you take over — otherwise the rail drags you back and is unusable.
     * Read from the drag interactions rather than `isScrollInProgress`, which is
     * also true during the rail's own programmatic scrolling.
     */
    var userIsBrowsing by remember { mutableStateOf(false) }

    LaunchedEffect(listState) {
        listState.interactionSource.interactions.collect { interaction ->
            if (interaction is DragInteraction.Start) userIsBrowsing = true
        }
    }

    // Follow the reader, so the rail shows where you are.
    LaunchedEffect(currentPage) {
        userIsBrowsing = false
        if (currentPage in 0 until pageCount) {
            listState.animateScrollToItem(currentPage)
        }
    }

    // ...and keep following it until it actually lands.
    //
    // Every cell starts at a guessed A4 aspect and resizes once its real page
    // dimensions arrive. With a hundred-plus cells above the viewport, those
    // corrections accumulate and drag the anchor: the rail was landing a dozen
    // pages away from the one being read. This absorbs that, then gets out of the
    // way as soon as the target is on screen or you start browsing.
    LaunchedEffect(currentPage, pageCount) {
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.map { it.index } }
            .collect { visible ->
                if (userIsBrowsing) return@collect
                if (visible.isEmpty() || currentPage !in 0 until pageCount) return@collect
                if (currentPage !in visible) listState.scrollToItem(currentPage)
            }
    }

    LazyColumn(
        state = listState,
        modifier = modifier
            .width(THUMBNAIL_STRIP_WIDTH)
            .fillMaxHeight()
            .background(MaterialTheme.colorScheme.surfaceContainerLow),
        contentPadding = PaddingValues(vertical = 10.dp, horizontal = 8.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        items(count = pageCount) { index ->
            ThumbnailCell(
                pageIndex = index,
                isCurrent = index == currentPage,
                onClick = { onSelectPage(index) },
                pageSizeProvider = pageSizeProvider,
                renderer = renderer,
            )
        }
    }
}

@Composable
private fun ThumbnailCell(
    pageIndex: Int,
    isCurrent: Boolean,
    onClick: () -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
) {
    var pageSize by remember(pageIndex) { mutableStateOf<PageSize?>(null) }
    var bitmap by remember(pageIndex) { mutableStateOf<Bitmap?>(null) }

    // Only renders while this cell is composed, so a LazyColumn naturally limits
    // the work to the handful of thumbnails actually on screen.
    LaunchedEffect(pageIndex) {
        val size = pageSizeProvider(pageIndex) ?: return@LaunchedEffect
        pageSize = size
        // Cancelled when this cell scrolls out of the LazyColumn, which is what
        // lets the throttled renderer skip it entirely instead of drawing
        // something nobody will see.
        bitmap = renderer(pageIndex, RenderScale.thumbnailFor(size))
    }

    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(pageSize?.aspectRatio ?: DEFAULT_ASPECT)
                .clip(RoundedCornerShape(3.dp))
                .background(MaterialTheme.colorScheme.surface)
                .border(
                    width = if (isCurrent) 2.dp else 1.dp,
                    color = if (isCurrent) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.outlineVariant
                    },
                    shape = RoundedCornerShape(3.dp),
                )
                .clickable(onClick = onClick),
        ) {
            bitmap?.let { bmp ->
                Image(
                    bitmap = bmp.asImageBitmap(),
                    contentDescription = "Go to page ${pageIndex + 1}",
                    modifier = Modifier.fillMaxWidth(),
                    contentScale = ContentScale.FillWidth,
                    filterQuality = FilterQuality.Medium,
                )
            }
        }
        Text(
            text = "${pageIndex + 1}",
            style = MaterialTheme.typography.labelSmall,
            fontWeight = if (isCurrent) FontWeight.Bold else FontWeight.Normal,
            color = if (isCurrent) {
                MaterialTheme.colorScheme.primary
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
            modifier = Modifier.padding(top = 3.dp),
        )
    }
}

/** Shared with the reader, which must subtract it when measuring pages. */
val THUMBNAIL_STRIP_WIDTH = 104.dp
private const val DEFAULT_ASPECT = 595f / 842f
