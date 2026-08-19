package com.hsilighting.pagify.ui.components

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Draw
import androidx.compose.material.icons.filled.Highlight
import androidx.compose.material.icons.filled.HorizontalRule
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.ui.reader.CapturePreview

/**
 * The capture, full screen, with everything you can do to it.
 *
 * Full screen rather than a sheet because this is a workspace, not a menu: it is
 * where the picture is checked, drawn on and decided about, and a sheet gave the
 * picture a third of the display while the controls took the rest.
 *
 * The picture can be pinched to zoom and panned with two fingers — the same
 * gestures as the reader, and for the same reason: drawing a small arrow on a
 * dense page needs the picture bigger than the screen, and the export is at its
 * own resolution regardless of what the display is showing.
 *
 * One finger always draws. That is what makes the two-finger split necessary
 * rather than a nicety: a one-finger drag cannot mean both "draw" and "pan".
 */
@Composable
fun CaptureEditor(
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
    var zoom by remember { mutableFloatStateOf(1f) }
    var pan by remember { mutableStateOf(Offset.Zero) }

    // A window of its own rather than a box inside the reader, so it covers the
    // app bar and the thumbnail rail too. Placed in the reader's content area it
    // was "full screen" only in the part of the screen the reader already owned,
    // which left the document's title sitting above the picture.
    //
    // `usePlatformDefaultWidth = false` is what stops the platform sizing it like
    // an alert; without it the whole editor is inset to dialog width.
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            dismissOnClickOutside = false,
        ),
    ) {
        // Back closes the editor rather than the document. The dialog handles back
        // itself, but only for its own window — this keeps the two paths the same.
        BackHandler(onBack = onDismiss)

        Surface(
            modifier = Modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.surface,
        ) {
        Column(
            Modifier
                .fillMaxSize()
                .windowInsetsPadding(WindowInsets.statusBars)
                .windowInsetsPadding(WindowInsets.navigationBars),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(onClick = onDismiss) {
                        Icon(Icons.Filled.Close, contentDescription = "Discard the picture")
                    }
                    Column(Modifier.weight(1f)) {
                        Text("Picture", style = MaterialTheme.typography.titleMedium)
                        Text(
                            "Page ${preview.request.originPage + 1} · " +
                                "${preview.request.scale.label} · " +
                                "${preview.request.format.extension.uppercase()} · " +
                                preview.sizeLabel,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (zoom != 1f) {
                        // Only offered once it would do something. A reset button that
                        // is always there invites the question of what it resets.
                        IconButton(
                            onClick = {
                                zoom = 1f
                                pan = Offset.Zero
                            },
                        ) {
                            Text("1:1", style = MaterialTheme.typography.labelLarge)
                        }
                    }
                    IconButton(onClick = onUndoMarkup, enabled = markup.isNotEmpty()) {
                        Icon(Icons.AutoMirrored.Filled.Undo, contentDescription = "Undo the last mark")
                    }
                }
    
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .background(MaterialTheme.colorScheme.surfaceVariant)
                        // Two fingers zoom and pan; one finger is left alone so it
                        // reaches the drawing surface underneath. Both handlers claim
                        // events on the Initial pass once a second finger lands, which
                        // is what stops a pinch turning into a stroke.
                        .pinchToZoom { factor, _ ->
                            zoom = (zoom * factor).coerceIn(MINIMUM_ZOOM, MAXIMUM_ZOOM)
                            if (zoom == 1f) pan = Offset.Zero
                        }
                        .twoFingerPanXY { delta -> pan += delta },
                    contentAlignment = Alignment.Center,
                ) {
                    CaptureCanvas(
                        image = preview.preview,
                        crop = preview.request.localBounds,
                        markup = markup,
                        tool = markupTool,
                        color = markupColor,
                        onCommit = onCommitMarkup,
                        onRecognise = onRecogniseMarkup,
                        zoom = zoom,
                        pan = pan,
                        modifier = Modifier.fillMaxSize().padding(8.dp),
                    )
    
                    if (isCapturing) {
                        // Over the picture rather than instead of it: re-rendering at a
                        // higher scale takes a moment, and swapping the picture for a
                        // spinner reads as the capture having been lost.
                        Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.3f)),
                            contentAlignment = Alignment.Center,
                        ) {
                            CircularProgressIndicator()
                        }
                    }
                }
    
                Column(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    MarkupTools(
                        tool = markupTool,
                        color = markupColor,
                        onTool = onMarkupTool,
                        onColor = onMarkupColor,
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
                            Icon(
                                Icons.Filled.ContentCopy,
                                contentDescription = null,
                                Modifier.size(18.dp),
                            )
                            Text("  Copy")
                        }
                    }
                }
            }
        }
    }
}


/**
 * What to draw with.
 *
 * The pen is first and selected by default: it needs no aiming, and holding still
 * at the end of a stroke turns a rough circle or box into the tidy version anyway,
 * so the other tools are for when someone wants the shape without the hold.
 */
@Composable
private fun MarkupTools(
    tool: MarkupTool,
    color: Long,
    onTool: (MarkupTool) -> Unit,
    onColor: (Long) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
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

        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            MARKUP_COLOURS.forEach { swatch ->
                Box(
                    modifier = Modifier
                        .size(26.dp)
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
 * something out, and it has to hold up next to black text.
 */
private val MARKUP_COLOURS = listOf(
    AnnotationColors.RED,
    AnnotationColors.BLUE,
    AnnotationColors.GREEN,
    AnnotationColors.YELLOW,
    AnnotationColors.ORANGE,
    AnnotationColors.PINK,
)

/** Below 1× there is nothing more to see; the picture already fits. */
private const val MINIMUM_ZOOM = 1f

/**
 * Past this the preview is being magnified rather than resolved: it is decoded
 * downsampled, so more zoom only enlarges pixels.
 */
private const val MAXIMUM_ZOOM = 8f
