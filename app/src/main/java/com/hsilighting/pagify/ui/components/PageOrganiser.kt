package com.hsilighting.pagify.ui.components

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Redo
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.KeyboardArrowLeft
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.EditState
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.RenderScale
import kotlinx.coroutines.delay

/**
 * One thing the user can do to the page tree.
 *
 * A single sealed type rather than a callback per operation: the reader already
 * takes around thirty parameters, and every new page operation would otherwise add
 * another to the chain from screen to view model. It also mirrors the engine's own
 * `Command`, so the two lists can be read side by side.
 */
sealed interface PageAction {
    data class Delete(val index: Int) : PageAction
    data class InsertBlankAt(val at: Int) : PageAction
    data class Move(val from: Int, val to: Int) : PageAction
    data class Rotate(val index: Int) : PageAction
    data object Undo : PageAction
    data object Redo : PageAction
}

/**
 * Rearranging, rotating, deleting and adding pages.
 *
 * Pages move one step at a time with the arrow buttons rather than by dragging.
 * A drag-and-drop grid is the more obvious design and a worse one here: the
 * gesture competes with the sheet's own scrolling and with the swipe that
 * dismisses it, and on a tablet held in two hands a long drag across a 149-page
 * grid is genuinely hard to complete. Stepping is unglamorous, reliable, and each
 * step is separately undoable.
 *
 * Undo and redo are the *document's*, not the annotation history's — the two are
 * kept apart deliberately, so this sheet is where document history is shown.
 */
@Composable
fun PageOrganiser(
    pageCount: Int,
    currentPage: Int,
    editState: EditState,
    isSaving: Boolean,
    onAction: (PageAction) -> Unit,
    onSave: () -> Unit,
    /** Write to a file the user picks — the way out when the original is read-only. */
    onSaveCopy: () -> Unit,
    onClose: () -> Unit,
    /**
     * The reader's one-shot message, shown here rather than in the snackbar.
     *
     * A modal sheet draws over the `Scaffold`, and its snackbar host with it — so a
     * save that fails while this is open produced a message nobody could see. That
     * was measured, not guessed: saving a document opened from another app fails
     * with a `SecurityException` on a read-only grant, and the only sign of it was
     * in logcat.
     */
    message: String?,
    onMessageShown: () -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    modifier: Modifier = Modifier,
) {
    // A bottom sheet sizes itself to its content, so the column it hands us has no
    // bounded height — and a `weight` with nothing to divide up does nothing at all.
    // The grid then grew to fit every page and pushed Save and Close off the bottom
    // of the screen, where no amount of scrolling reached them. Capping the height
    // here is what gives the weight below something to work with.
    val maxSheetHeight = (LocalConfiguration.current.screenHeightDp * SHEET_HEIGHT_FRACTION).dp

    Column(
        modifier = modifier
            .heightIn(max = maxSheetHeight)
            .padding(horizontal = 16.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Organise pages", style = MaterialTheme.typography.titleMedium)
                Text(
                    text = buildString {
                        append(if (pageCount == 1) "1 page" else "$pageCount pages")
                        if (editState.dirty) append(" · unsaved changes")
                    },
                    style = MaterialTheme.typography.labelMedium,
                )
            }

            IconButton(
                onClick = { onAction(PageAction.Undo) },
                enabled = editState.canUndo && !isSaving,
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.Undo,
                    // Naming the specific change is what makes undo safe to press:
                    // "Undo" alone gives no way to tell what is about to be reversed.
                    contentDescription = editState.undoLabel?.let { "Undo: $it" }
                        ?: "Undo the last page change",
                )
            }
            IconButton(
                onClick = { onAction(PageAction.Redo) },
                enabled = editState.canRedo && !isSaving,
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.Redo,
                    contentDescription = editState.redoLabel?.let { "Redo: $it" }
                        ?: "Redo the last undone page change",
                )
            }
        }

        if (!editState.editable) {
            Text(
                text = "This document cannot be edited.",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(vertical = 12.dp),
            )
        }

        LazyVerticalGrid(
            columns = GridCells.Adaptive(minSize = 132.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            // `fill = false` so a two-page document does not stretch the sheet to
            // full height with empty space under the grid; the cap above is what
            // stops a long document pushing the action row out of reach.
            modifier = Modifier
                .weight(1f, fill = false)
                .padding(vertical = 12.dp),
        ) {
            items(
                // A key on the index alone would let a deleted page's thumbnail
                // stay attached to whatever moves into its position.
                items = (0 until pageCount).toList(),
                key = { it },
            ) { index ->
                PageCell(
                    index = index,
                    isCurrent = index == currentPage,
                    // Any edit invalidates every thumbnail here: rotating page 3
                    // changes how it draws, and deleting page 3 changes what page 4
                    // *is*. Keyed on the whole state rather than the page count,
                    // which a rotation does not change at all — that left rotated
                    // pages showing their old orientation until the sheet was
                    // reopened.
                    revision = editState,
                    enabled = editState.editable && !isSaving,
                    canMoveLeft = index > 0,
                    canMoveRight = index < pageCount - 1,
                    canDelete = pageCount > 1,
                    onAction = onAction,
                    pageSizeProvider = pageSizeProvider,
                    renderer = renderer,
                )
            }
        }

        if (message != null) {
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(bottom = 8.dp),
            )
            // Cleared on a timer so the next message is seen as a new one rather
            // than blending into the last.
            LaunchedEffect(message) {
                delay(MESSAGE_DWELL_MILLIS)
                onMessageShown()
            }
        }

        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(bottom = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(
                onClick = { onAction(PageAction.InsertBlankAt(currentPage + 1)) },
                enabled = editState.editable && !isSaving,
            ) {
                Icon(Icons.Filled.Add, contentDescription = null)
                Text("Blank page", modifier = Modifier.padding(start = 8.dp))
            }

            Box(modifier = Modifier.weight(1f))

            if (isSaving) {
                CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp))
            }
            TextButton(onClick = onSaveCopy, enabled = editState.dirty && !isSaving) {
                Text("Save a copy")
            }
            TextButton(onClick = onClose, enabled = !isSaving) { Text("Close") }
            FilledTonalButton(onClick = onSave, enabled = editState.dirty && !isSaving) {
                Text("Save")
            }
        }
    }
}

@Composable
private fun PageCell(
    index: Int,
    isCurrent: Boolean,
    /** Changes whenever the document does, so the thumbnail is re-rendered. */
    revision: Any,
    enabled: Boolean,
    canMoveLeft: Boolean,
    canMoveRight: Boolean,
    canDelete: Boolean,
    onAction: (PageAction) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
) {
    var bitmap by remember(index, revision) { mutableStateOf<Bitmap?>(null) }

    LaunchedEffect(index, revision) {
        val size = pageSizeProvider(index) ?: return@LaunchedEffect
        bitmap = renderer(index, RenderScale.thumbnailFor(size))
    }

    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(0.72f)
                .clip(RoundedCornerShape(4.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .border(
                    width = if (isCurrent) 2.dp else 1.dp,
                    color = if (isCurrent) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.outlineVariant
                    },
                    shape = RoundedCornerShape(4.dp),
                )
                .semantics { contentDescription = "Page ${index + 1}" },
            contentAlignment = Alignment.Center,
        ) {
            bitmap?.let {
                androidx.compose.foundation.Image(
                    bitmap = it.asImageBitmap(),
                    contentDescription = null,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }

        Text("${index + 1}", style = MaterialTheme.typography.labelSmall)

        Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
            IconButton(
                onClick = { onAction(PageAction.Move(index, index - 1)) },
                enabled = enabled && canMoveLeft,
            ) {
                Icon(
                    Icons.Filled.KeyboardArrowLeft,
                    contentDescription = "Move page ${index + 1} earlier",
                )
            }
            IconButton(
                onClick = { onAction(PageAction.Rotate(index)) },
                enabled = enabled,
            ) {
                Icon(Icons.Filled.Refresh, contentDescription = "Rotate page ${index + 1}")
            }
            IconButton(
                onClick = { onAction(PageAction.Delete(index)) },
                enabled = enabled && canDelete,
            ) {
                Icon(Icons.Filled.Delete, contentDescription = "Delete page ${index + 1}")
            }
            IconButton(
                onClick = { onAction(PageAction.Move(index, index + 1)) },
                enabled = enabled && canMoveRight,
            ) {
                Icon(
                    Icons.Filled.KeyboardArrowRight,
                    contentDescription = "Move page ${index + 1} later",
                )
            }
        }
    }
}

/** How long a message stays on screen in the sheet before it is cleared. */
private const val MESSAGE_DWELL_MILLIS = 4_000L

/**
 * How much of the screen the sheet may occupy.
 *
 * Leaves the document visible behind it, which is what makes the sheet feel like
 * a panel over the reader rather than a separate screen.
 */
private const val SHEET_HEIGHT_FRACTION = 0.82f
