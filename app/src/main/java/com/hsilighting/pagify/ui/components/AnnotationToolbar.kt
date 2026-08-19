package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Backspace
import androidx.compose.material.icons.filled.Brush
import androidx.compose.material.icons.filled.CropFree
import androidx.compose.material.icons.filled.Draw
import androidx.compose.material.icons.filled.HistoryEdu
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PenMode

/**
 * The floating tool ribbon along the bottom of the reader.
 *
 * Tapping a tool selects it, and tapping the selected tool again puts the reader
 * back to plain scrolling — a tool that can only be turned on is a trap, since
 * every touch would keep drawing.
 *
 * The pen additionally opens a palette on long press, carrying both the colour
 * and the choice between highlighting text and free drawing. Those belong
 * together: they are the two ways the same pen behaves, and separating them into
 * two ribbon slots would imply they can both be active.
 */
@Composable
fun AnnotationToolbar(
    selectedTool: AnnotationTool,
    penMode: PenMode,
    penColor: Long,
    onSelectTool: (AnnotationTool) -> Unit,
    onPenModeChange: (PenMode) -> Unit,
    onPenColorChange: (Long) -> Unit,
    /** Marks on the page being read, and in the document as a whole. */
    marksOnPage: Int,
    marksInDocument: Int,
    onClearPage: () -> Unit,
    onClearAll: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var showPenPalette by remember { mutableStateOf(false) }
    var showClearMenu by remember { mutableStateOf(false) }

    Box(modifier, contentAlignment = Alignment.BottomCenter) {
        if (showPenPalette) {
            PenPalette(
                penMode = penMode,
                penColor = penColor,
                onPenModeChange = onPenModeChange,
                onPenColorChange = onPenColorChange,
                onDismiss = { showPenPalette = false },
                modifier = Modifier.padding(bottom = 74.dp),
            )
        }
        if (showClearMenu) {
            ClearMenu(
                marksOnPage = marksOnPage,
                marksInDocument = marksInDocument,
                onClearPage = onClearPage,
                onClearAll = onClearAll,
                onDismiss = { showClearMenu = false },
                modifier = Modifier.padding(bottom = 74.dp),
            )
        }

        Surface(
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            shadowElevation = 6.dp,
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ToolButton(
                    icon = if (penMode == PenMode.Marker) Icons.Filled.Brush else Icons.Filled.Draw,
                    label = if (penMode == PenMode.Marker) "Marker" else "Highlighter",
                    selected = selectedTool == AnnotationTool.Pen,
                    accent = Color(penColor),
                    onClick = {
                        onSelectTool(
                            if (selectedTool == AnnotationTool.Pen) {
                                AnnotationTool.None
                            } else {
                                AnnotationTool.Pen
                            },
                        )
                    },
                    onLongClick = {
                        // Long press opens the palette and selects the pen, so the
                        // colour you just picked is immediately the one in use.
                        onSelectTool(AnnotationTool.Pen)
                        showPenPalette = true
                    },
                )
                ToolButton(
                    icon = Icons.Filled.TextFields,
                    label = "Note",
                    selected = selectedTool == AnnotationTool.Note,
                    onClick = { onSelectTool(toggle(selectedTool, AnnotationTool.Note)) },
                )
                ToolButton(
                    // Distinct from the pen on purpose: both were Draw, and two
                    // identical glyphs in a four-slot ribbon is unreadable.
                    icon = Icons.Filled.HistoryEdu,
                    label = "Signature",
                    selected = selectedTool == AnnotationTool.Signature,
                    onClick = { onSelectTool(toggle(selectedTool, AnnotationTool.Signature)) },
                )
                ToolButton(
                    icon = Icons.Filled.Backspace,
                    label = "Eraser",
                    selected = selectedTool == AnnotationTool.Eraser,
                    onClick = { onSelectTool(toggle(selectedTool, AnnotationTool.Eraser)) },
                    onLongClick = {
                        // Clearing a page or the document lives behind the eraser
                        // because that is what it means — the same action, wider.
                        // Long press does not select the tool, so a wipe is not
                        // followed by an armed eraser under your finger.
                        showClearMenu = true
                    },
                )
                ToolButton(
                    icon = Icons.Filled.CropFree,
                    label = "Snapshot",
                    selected = selectedTool == AnnotationTool.Snapshot,
                    onClick = { onSelectTool(toggle(selectedTool, AnnotationTool.Snapshot)) },
                )
            }
        }
    }
}

private fun toggle(current: AnnotationTool, tapped: AnnotationTool): AnnotationTool =
    if (current == tapped) AnnotationTool.None else tapped

@Composable
private fun ToolButton(
    icon: ImageVector,
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    accent: Color? = null,
    onLongClick: (() -> Unit)? = null,
) {
    Box(
        modifier = modifier
            .size(46.dp)
            .clip(CircleShape)
            .background(
                if (selected) MaterialTheme.colorScheme.primary else Color.Transparent,
            )
            .then(
                if (onLongClick != null) {
                    Modifier.combinedClickableCompat(onClick = onClick, onLongClick = onLongClick)
                } else {
                    Modifier.clickable(onClick = onClick)
                },
            ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = label,
            tint = when {
                selected -> MaterialTheme.colorScheme.onPrimary
                else -> MaterialTheme.colorScheme.onSurfaceVariant
            },
            modifier = Modifier.size(22.dp),
        )
        // A dot of the current ink, so the pen's colour is visible without
        // opening the palette.
        if (accent != null) {
            Box(
                Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 6.dp)
                    .size(6.dp)
                    .clip(CircleShape)
                    .background(accent),
            )
        }
    }
}

@Composable
private fun PenPalette(
    penMode: PenMode,
    penColor: Long,
    onPenModeChange: (PenMode) -> Unit,
    onPenColorChange: (Long) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val palette = when (penMode) {
        PenMode.Highlight -> AnnotationColors.highlightPalette
        PenMode.Marker -> AnnotationColors.markerPalette
    }

    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Row(
            Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            ModeChip(
                label = "Highlight",
                selected = penMode == PenMode.Highlight,
                onClick = { onPenModeChange(PenMode.Highlight) },
            )
            ModeChip(
                label = "Marker",
                selected = penMode == PenMode.Marker,
                onClick = { onPenModeChange(PenMode.Marker) },
            )

            palette.forEach { color ->
                Box(
                    Modifier
                        .size(30.dp)
                        .clip(CircleShape)
                        .background(Color(color))
                        .border(
                            width = if (color == penColor) 3.dp else 1.dp,
                            color = if (color == penColor) {
                                MaterialTheme.colorScheme.primary
                            } else {
                                MaterialTheme.colorScheme.outlineVariant
                            },
                            shape = CircleShape,
                        )
                        .clickable {
                            onPenColorChange(color)
                            onDismiss()
                        },
                )
            }
        }
    }
}

@Composable
private fun ModeChip(label: String, selected: Boolean, onClick: () -> Unit) {
    Surface(
        shape = RoundedCornerShape(14.dp),
        color = if (selected) {
            MaterialTheme.colorScheme.primary
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        },
        modifier = Modifier.clickable(onClick = onClick),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = if (selected) {
                MaterialTheme.colorScheme.onPrimary
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 7.dp),
        )
    }
}

/**
 * The wider erasures, behind a long press on the eraser.
 *
 * Both confirm before they run. Undo would bring the marks back either way, but a
 * wipe you did not mean to trigger is alarming in a way a single erased highlight
 * is not, and the count in the prompt is what tells you which of the two actions
 * you are about to take.
 */
@Composable
private fun ClearMenu(
    marksOnPage: Int,
    marksInDocument: Int,
    onClearPage: () -> Unit,
    onClearAll: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var confirming by remember { mutableStateOf<ClearScope?>(null) }

    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Row(
            Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            ClearChip(
                label = "Clear page ($marksOnPage)",
                enabled = marksOnPage > 0,
                onClick = { confirming = ClearScope.Page },
            )
            ClearChip(
                label = "Clear all ($marksInDocument)",
                enabled = marksInDocument > 0,
                onClick = { confirming = ClearScope.Document },
            )
        }
    }

    confirming?.let { scope ->
        val count = if (scope == ClearScope.Page) marksOnPage else marksInDocument
        val where = if (scope == ClearScope.Page) "this page" else "the whole document"
        AlertDialog(
            onDismissRequest = { confirming = null },
            title = { Text(if (scope == ClearScope.Page) "Clear this page?" else "Clear everything?") },
            text = {
                Text(
                    "This removes ${count.marks()} from $where. " +
                        "You can undo it.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        if (scope == ClearScope.Page) onClearPage() else onClearAll()
                        confirming = null
                        onDismiss()
                    },
                ) { Text("Clear") }
            },
            dismissButton = {
                TextButton(onClick = { confirming = null }) { Text("Cancel") }
            },
        )
    }
}

private enum class ClearScope { Page, Document }

private fun Int.marks(): String = if (this == 1) "1 mark" else "$this marks"

@Composable
private fun ClearChip(label: String, enabled: Boolean, onClick: () -> Unit) {
    Surface(
        shape = RoundedCornerShape(14.dp),
        color = if (enabled) {
            MaterialTheme.colorScheme.errorContainer
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        },
        modifier = Modifier.clickable(enabled = enabled, onClick = onClick),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = if (enabled) {
                MaterialTheme.colorScheme.onErrorContainer
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
            },
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
        )
    }
}

/**
 * Shown when the highlighter is active on a page with no selectable text.
 *
 * Two quite different documents land here and the wording must fit both. A scan
 * is an image of a page. Far more common in practice is type **converted to
 * outlines** — the words drawn as vector paths by a print export — which reads
 * perfectly, renders crisply at any zoom, and carries no characters at all. The
 * 2.9 GB catalogue is the second kind: 92 of its 95 pages have zero text objects
 * while being pure vector artwork, which is why it looks like text that ought to
 * be selectable and is not, in this viewer or any other.
 *
 * Neither can be highlighted without OCR, so the message says what is true of
 * both rather than guessing which one this is. It offers the marker instead of
 * only reporting the problem, because drawing on the page is what the reader was
 * trying to do.
 */
@Composable
fun NoTextOnPageHint(
    onUseMarker: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Row(
            Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "No selectable text on this page",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            TextButton(onClick = onUseMarker) { Text("Use marker") }
            TextButton(onClick = onDismiss) { Text("Dismiss") }
        }
    }
}

/**
 * Shown while a page is being read by the recogniser.
 *
 * The wording says *reading*, not "loading": the text is being worked out from
 * the picture of the page, and it may come back imperfect. Setting that
 * expectation here is cheaper than explaining a wrong character later.
 */
@Composable
fun RecognisingTextHint(modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Row(
            Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            CircularProgressIndicator(
                modifier = Modifier.size(18.dp),
                strokeWidth = 2.dp,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            Text(
                text = "Reading text on this page…",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
        }
    }
}
