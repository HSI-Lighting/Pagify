package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Draw
import androidx.compose.material.icons.filled.HorizontalRule
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Highlight
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.ui.reader.CapturePreview
import androidx.compose.ui.geometry.Offset

/**
 * What was captured, what to draw on it, and what to do with it.
 *
 * Shown after the region is taken rather than before, because every choice here
 * is easier to make while looking at the result: whether the crop caught what was
 * wanted, whether it needs to be sharper, whether a photograph would be better as
 * a JPEG. Changing the scale or the format re-renders from the same crop, so
 * nothing has to be dragged out again — and the marks, being in page points,
 * survive that unchanged.
 */
@Composable
fun CaptureSheet(
    preview: CapturePreview,
    isCapturing: Boolean,
    markup: List<Markup>,
    markupTool: MarkupTool,
    markupColor: Long,
    onScaleChange: (CaptureScale) -> Unit,
    onFormatChange: (CaptureFormat) -> Unit,
    onMarkupTool: (MarkupTool) -> Unit,
    onMarkupColor: (Long) -> Unit,
    onCommitMarkup: (MarkupShape) -> Unit,
    onRecogniseMarkup: (List<Offset>) -> Unit,
    onUndoMarkup: () -> Unit,
    onSaveToGallery: () -> Unit,
    onShare: () -> Unit,
    onCopy: () -> Unit,
    onDismiss: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp)
            .padding(bottom = 28.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Picture taken", style = MaterialTheme.typography.titleLarge)
            IconButton(onClick = onUndoMarkup, enabled = markup.isNotEmpty()) {
                Icon(Icons.AutoMirrored.Filled.Undo, contentDescription = "Undo the last mark")
            }
        }

        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(300.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) {
            CaptureCanvas(
                image = preview.preview,
                crop = preview.request.crop,
                markup = markup,
                tool = markupTool,
                color = markupColor,
                onCommit = onCommitMarkup,
                onRecognise = onRecogniseMarkup,
                modifier = Modifier.fillMaxWidth().height(300.dp),
            )
            // Over the picture rather than instead of it: a re-render at a higher
            // scale takes a moment, and swapping the picture for a spinner reads
            // as the capture having been lost.
            if (isCapturing) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(300.dp)
                        .background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.35f)),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator()
                }
            }
        }

        MarkupTools(
            tool = markupTool,
            color = markupColor,
            onTool = onMarkupTool,
            onColor = onMarkupColor,
        )

        Text(
            "Page ${preview.request.pageIndex + 1} · ${preview.request.scale.label} · " +
                "${preview.request.format.extension.uppercase()} · ${preview.sizeLabel}",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            CaptureScale.entries.forEach { scale ->
                FilterChip(
                    selected = preview.request.scale == scale,
                    onClick = { onScaleChange(scale) },
                    enabled = !isCapturing,
                    label = { Text(scale.label) },
                )
            }
            CaptureFormat.entries.forEach { format ->
                FilterChip(
                    selected = preview.request.format == format,
                    onClick = { onFormatChange(format) },
                    enabled = !isCapturing,
                    label = { Text(format.extension.uppercase()) },
                )
            }
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(
                onClick = onSaveToGallery,
                enabled = !isCapturing,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.Download, contentDescription = null, Modifier.size(18.dp))
                Text("  Save")
            }
            OutlinedButton(
                onClick = onShare,
                enabled = !isCapturing,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.Share, contentDescription = null, Modifier.size(18.dp))
                Text("  Share")
            }
            OutlinedButton(
                onClick = onCopy,
                enabled = !isCapturing,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.ContentCopy, contentDescription = null, Modifier.size(18.dp))
                Text("  Copy")
            }
        }

        TextButton(onClick = onDismiss, modifier = Modifier.align(Alignment.End)) {
            Text("Discard")
        }
    }
}

/**
 * What to draw with.
 *
 * The pen is first and selected by default: it is the one that needs no aiming,
 * and the recogniser turns a held circle or box into the tidy version anyway, so
 * the other tools are for when someone wants the shape without the hold.
 */
@Composable
private fun MarkupTools(
    tool: MarkupTool,
    color: Long,
    onTool: (MarkupTool) -> Unit,
    onColor: (Long) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            MarkupToolButton(Icons.Filled.Draw, "Pen", MarkupTool.Pen, tool, onTool)
            MarkupToolButton(
                Icons.Filled.HorizontalRule,
                "Line",
                MarkupTool.Line,
                tool,
                onTool,
            )
            MarkupToolButton(
                Icons.AutoMirrored.Filled.ArrowRightAlt,
                "Arrow",
                MarkupTool.Arrow,
                tool,
                onTool,
            )
            MarkupToolButton(
                Icons.Filled.CheckBoxOutlineBlank,
                "Box",
                MarkupTool.Rectangle,
                tool,
                onTool,
            )
            MarkupToolButton(
                Icons.Filled.RadioButtonUnchecked,
                "Circle",
                MarkupTool.Ellipse,
                tool,
                onTool,
            )
            MarkupToolButton(
                Icons.Filled.Highlight,
                "Highlight",
                MarkupTool.Highlight,
                tool,
                onTool,
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            MARKUP_COLOURS.forEach { swatch ->
                Box(
                    modifier = Modifier
                        .size(28.dp)
                        .background(Color(swatch), CircleShape)
                        .border(
                            width = if (swatch == color) 3.dp else 1.dp,
                            color = if (swatch == color) {
                                MaterialTheme.colorScheme.onSurface
                            } else {
                                MaterialTheme.colorScheme.outlineVariant
                            },
                            shape = CircleShape,
                        )
                        .clickable { onColor(swatch) },
                )
            }
        }
    }
}

@Composable
private fun MarkupToolButton(
    icon: ImageVector,
    label: String,
    represents: MarkupTool,
    selected: MarkupTool,
    onTool: (MarkupTool) -> Unit,
) {
    val isSelected = represents == selected
    IconButton(
        onClick = { onTool(represents) },
        modifier = Modifier
            .size(44.dp)
            .background(
                color = if (isSelected) {
                    MaterialTheme.colorScheme.secondaryContainer
                } else {
                    Color.Transparent
                },
                shape = CircleShape,
            ),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = label,
            tint = if (isSelected) {
                MaterialTheme.colorScheme.onSecondaryContainer
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
        )
    }
}

/**
 * Ink colours.
 *
 * Red first, and the default: markup on a page is almost always pointing
 * something out, and it has to survive being seen next to black text.
 */
private val MARKUP_COLOURS = listOf(
    AnnotationColors.RED,
    AnnotationColors.BLUE,
    AnnotationColors.GREEN,
    AnnotationColors.YELLOW,
    AnnotationColors.ORANGE,
    AnnotationColors.PINK,
)
