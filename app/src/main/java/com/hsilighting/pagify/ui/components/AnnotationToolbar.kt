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
import androidx.compose.material.icons.filled.Brush
import androidx.compose.material.icons.filled.CropFree
import androidx.compose.material.icons.filled.Draw
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
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
    modifier: Modifier = Modifier,
) {
    var showPenPalette by remember { mutableStateOf(false) }

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
                    icon = Icons.Filled.Draw,
                    label = "Signature",
                    selected = selectedTool == AnnotationTool.Signature,
                    onClick = { onSelectTool(toggle(selectedTool, AnnotationTool.Signature)) },
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
