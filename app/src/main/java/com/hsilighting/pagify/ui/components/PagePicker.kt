package com.hsilighting.pagify.ui.components

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.Image
import com.hsilighting.pagify.core.PageSize

/**
 * Choosing pages out of a document.
 *
 * Takes its pages as lambdas rather than a document, because it is used against
 * two different ones: the document being read, when choosing what to export, and
 * a file just opened, when choosing what to import from it. Neither knows about
 * the other.
 *
 * Selection is a *list*, not a set. The order pages are tapped in is the order
 * they come out in — "give me page 3, then page 1" is a thing somebody can ask
 * for, and quietly sorting it hands them a different document.
 */
@Composable
fun PagePicker(
    pageCount: Int,
    /** What the button says, e.g. "Export 3 pages". */
    confirmLabel: (Int) -> String,
    onConfirm: (List<Int>) -> Unit,
    onCancel: () -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
) {
    var chosen by remember(pageCount) { mutableStateOf(listOf<Int>()) }

    Column(modifier.padding(horizontal = 16.dp)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = if (chosen.isEmpty()) {
                    "Tap the pages you want"
                } else {
                    "${chosen.size} of $pageCount chosen"
                },
                style = MaterialTheme.typography.titleMedium,
            )
            TextButton(
                onClick = {
                    chosen = if (chosen.size == pageCount) {
                        emptyList()
                    } else {
                        (0 until pageCount).toList()
                    }
                },
            ) { Text(if (chosen.size == pageCount) "None" else "All") }
        }

        LazyVerticalGrid(
            columns = GridCells.Adaptive(minSize = 108.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
            // `fill = false` so a two-page document does not stretch the sheet to
            // full height with empty space under the grid.
            modifier = Modifier
                .weight(1f, fill = false)
                .padding(vertical = 12.dp),
        ) {
            items(items = (0 until pageCount).toList(), key = { it }) { index ->
                val order = chosen.indexOf(index)
                PickerCell(
                    index = index,
                    // The position in the chosen list, not a tick: with order
                    // mattering, "3rd" is the fact somebody needs to see, and a
                    // tick would hide it.
                    order = if (order >= 0) order + 1 else null,
                    onTap = {
                        chosen = if (order >= 0) chosen - index else chosen + index
                    },
                    pageSizeProvider = pageSizeProvider,
                    renderer = renderer,
                )
            }
        }

        Row(
            Modifier
                .fillMaxWidth()
                .padding(bottom = 12.dp),
            horizontalArrangement = Arrangement.End,
        ) {
            TextButton(onClick = onCancel) { Text("Cancel") }
            TextButton(
                enabled = chosen.isNotEmpty(),
                onClick = { onConfirm(chosen) },
            ) { Text(confirmLabel(chosen.size)) }
        }
    }
}

@Composable
private fun PickerCell(
    index: Int,
    order: Int?,
    onTap: () -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
) {
    var bitmap by remember(index) { mutableStateOf<Bitmap?>(null) }
    var ratio by remember(index) { mutableStateOf(DEFAULT_PAGE_RATIO) }

    LaunchedEffect(index) {
        val size = pageSizeProvider(index) ?: return@LaunchedEffect
        if (size.widthPoints > 0f && size.heightPoints > 0f) {
            ratio = size.widthPoints / size.heightPoints
        }
        bitmap = renderer(index, THUMBNAIL_ZOOM)
    }

    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(ratio)
                .clip(RoundedCornerShape(6.dp))
                .background(Color.White)
                .border(
                    width = if (order != null) 3.dp else 1.dp,
                    color = if (order != null) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.outlineVariant
                    },
                    shape = RoundedCornerShape(6.dp),
                )
                .clickable(onClick = onTap),
        ) {
            bitmap?.let {
                Image(
                    bitmap = it.asImageBitmap(),
                    contentDescription = null,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            if (order != null) {
                Box(
                    Modifier
                        .align(Alignment.TopEnd)
                        .padding(4.dp)
                        .size(22.dp)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.primary),
                    contentAlignment = Alignment.Center,
                ) {
                    if (order > MOST_SHOWN_AS_A_NUMBER) {
                        Icon(
                            Icons.Filled.Check,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onPrimary,
                            modifier = Modifier.size(14.dp),
                        )
                    } else {
                        Text(
                            text = "$order",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onPrimary,
                        )
                    }
                }
            }
        }
        Text(
            text = "${index + 1}",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

/** A4 upright, for the moment before the real page size is known. */
private const val DEFAULT_PAGE_RATIO = 595f / 842f

/** Small enough to be quick, large enough to tell two pages apart. */
private const val THUMBNAIL_ZOOM = 0.35f

/**
 * Past this, the badge shows a tick instead of a number.
 *
 * Two digits fit; three do not, and a "12" squeezed to illegibility is worse than
 * a tick. Somebody selecting a hundred pages is selecting all of them anyway.
 */
private const val MOST_SHOWN_AS_A_NUMBER = 99
