package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.ui.zIndex
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.draw.scale
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.layout.offset
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
    /**
     * Point at a page.
     *
     * Which matters because it is where things go: a blank page and an import
     * both land after the page in hand. Without this the only page that could
     * be pointed at was whichever one the reader happened to be on behind the
     * sheet, so choosing where to put something meant closing the organiser,
     * scrolling the document, and opening it again.
     */
    data class Select(val index: Int) : PageAction

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
    /**
     * Marks made this session that are not yet in the file.
     *
     * Kept separate from [editState] because a highlight does not make the
     * *document* dirty — marks live in an `AnnotationStore` until a save writes
     * them out. Without this the Save button stayed disabled on a document whose
     * only change was every annotation the reader had drawn on it.
     */
    unsavedMarks: Int,
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
    /** Choose pages to write out as a PDF of their own. */
    /**
     * Bumped whenever the rendered pages stop being what the document holds.
     *
     * What the thumbnails key on. [editState] was doing this job and is the wrong
     * thing for it: it describes the *history*, so two edits that leave the same
     * undo label leave it unchanged, and the grid then goes on showing pages
     * where they used to be. This counter exists for exactly this and changes on
     * every invalidation, whatever the edit was.
     */
    pageContentRevision: Int,
    /** Choose pages to write out as a PDF of their own. */
    onExportPages: () -> Unit,
    /** Bring pages in from another PDF. */
    onImportPages: () -> Unit,
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
                        if (editState.dirty || unsavedMarks > 0) append(" · unsaved changes")
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

Text(
            text = "Tap a page to put things after it · hold to drag it",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        if (!editState.editable) {
            Text(
                text = "This document cannot be edited.",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(vertical = 12.dp),
            )
        }

        val gridState = rememberLazyGridState()
        val reorder = rememberGridReorderState(gridState) { from, to ->
            onAction(PageAction.Move(from, to))
        }

        // The order the grid draws. Identity except while a page is being
        // dragged, when it is what the drop would produce — so the pages shuffle
        // under the finger and the result is visible before letting go.
        val order = reorder.order.takeIf { it.size == pageCount }
            ?: (0 until pageCount).toList()

        LazyVerticalGrid(
            state = gridState,
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
            itemsIndexed(
                // Keyed by the **slot**, not the page, and that is load-bearing.
                //
                // A lazy grid anchors its scroll position to the key of the first
                // visible item: when the list changes it finds that key again and
                // scrolls so the item stays put. Keying by page turns that against
                // us. Dragging page 1 downwards moves its key down the order, and
                // the grid dutifully chases it — so the viewport ran away down the
                // document, faster the further the page was dragged. It measured
                // as pages 10 to 45 in 78 milliseconds, with the drag scroll doing
                // nothing at all: the grid was following its own anchor.
                //
                // Keyed by slot, the anchor stays where it is and only the content
                // moves. Re-rendering a shuffled cell costs nothing, because every
                // thumbnail it could want is already in the cache.
                items = order,
                key = { slot, _ -> slot },
            ) { slot, index ->
                PageCell(
                    index = index,
                    // What it would be numbered if the drag ended here. Outside a
                    // drag this is the page number; during one it is the answer to
                    // "where am I putting this".
                    label = slot + 1,
                    dragging = reorder.isDragging(index),
                    displacement = reorder.displacement(index),
                    reorderModifier = Modifier.reorderable(
                        state = reorder,
                        slot = slot,
                        count = pageCount,
                        enabled = editState.editable && !isSaving,
                    ),
                    isCurrent = index == currentPage,
                    // Any edit invalidates every thumbnail here: rotating page 3
                    // changes how it draws, and deleting page 3 changes what page 4
                    // *is*. Keyed on the whole state rather than the page count,
                    // which a rotation does not change at all — that left rotated
                    // pages showing their old orientation until the sheet was
                    // reopened.
                    revision = pageContentRevision,
                    enabled = editState.editable && !isSaving,
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
                Text("Blank", modifier = Modifier.padding(start = 8.dp))
            }
            TextButton(onClick = onImportPages, enabled = editState.editable && !isSaving) {
                Icon(Icons.Filled.Add, contentDescription = null)
                Text("Import", modifier = Modifier.padding(start = 8.dp))
            }
            // Available on a read-only document too: writing chosen pages
            // somewhere else is exactly what you do when you cannot write
            // where they are.
            TextButton(onClick = onExportPages, enabled = !isSaving && pageCount > 0) {
                Text("Export")
            }

            Box(modifier = Modifier.weight(1f))

            if (isSaving) {
                CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp))
            }
            val hasChanges = editState.dirty || unsavedMarks > 0
            TextButton(onClick = onSaveCopy, enabled = hasChanges && !isSaving) {
                Text("Save a copy")
            }
            TextButton(onClick = onClose, enabled = !isSaving) { Text("Close") }
            FilledTonalButton(onClick = onSave, enabled = hasChanges && !isSaving) {
                Text("Save")
            }
        }
    }
}

@Composable
private fun PageCell(
    index: Int,
    /** The number to show: where this page would sit if a drag ended now. */
    label: Int,
    dragging: Boolean,
    displacement: IntOffset,
    reorderModifier: Modifier,
    isCurrent: Boolean,
    /** Changes whenever the document does, so the thumbnail is re-rendered. */
    revision: Any,
    enabled: Boolean,
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

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        // Lifted out of the flow and drawn last, so the page being dragged
        // passes over its neighbours rather than under them.
        modifier = Modifier
            .zIndex(if (dragging) 1f else 0f)
            .offset { displacement },
    ) {
        Box(
            modifier = reorderModifier
                // After the drag handler, so a long press that becomes a drag
                // is consumed there and never arrives here as a tap.
                .clickable { onAction(PageAction.Select(index)) }
                .fillMaxWidth()
                .aspectRatio(0.72f)
                .scale(if (dragging) LIFTED else 1f)
                .clip(RoundedCornerShape(4.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .border(
                    width = if (isCurrent || dragging) 2.dp else 1.dp,
                    color = if (isCurrent || dragging) {
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

        Text("$label", style = MaterialTheme.typography.labelSmall)

        Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
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

/** How much a page grows while it is held, so it reads as picked up. */
private const val LIFTED = 1.06f
