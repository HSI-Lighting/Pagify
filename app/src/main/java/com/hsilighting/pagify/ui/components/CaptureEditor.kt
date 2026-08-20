package com.hsilighting.pagify.ui.components

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.material3.Slider
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
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.isIntensity
import com.hsilighting.pagify.core.sizePresets
import com.hsilighting.pagify.core.sizeRange
import com.hsilighting.pagify.ui.reader.CapturePreview
import kotlin.math.roundToInt

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
    /** How heavy the current tool draws: nib width, or intensity for the wash. */
    markupSize: Float,
    onScaleChange: (CaptureScale) -> Unit,
    onFormatChange: (CaptureFormat) -> Unit,
    /** What fills the picture where no page reaches. */
    fill: CaptureFill,
    onFillChange: (CaptureFill) -> Unit,
    onMarkupTool: (MarkupTool) -> Unit,
    onMarkupColor: (Long) -> Unit,
    onMarkupSize: (MarkupTool, Float) -> Unit,
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
    var pickingColour by remember { mutableStateOf(false) }

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

        if (pickingColour) {
            ColourWheelDialog(
                initial = markupColor,
                onPick = {
                    onMarkupColor(it)
                    pickingColour = false
                },
                onDismiss = { pickingColour = false },
            )
        }

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
                        // A checkerboard, and only for a cut-out. Over a plain grey
                        // panel "transparent" and "the reader's grey backdrop" look
                        // exactly alike, and the whole reason to pick transparent is
                        // that it is *not* a colour.
                        .then(
                            if (fill == CaptureFill.TRANSPARENT) {
                                Modifier.drawBehind { drawCheckerboard() }
                            } else {
                                Modifier
                            },
                        )
                        // Two fingers zoom and pan; one finger is left alone so it
                        // reaches the drawing surface underneath. Both handlers claim
                        // events on the Initial pass once a second finger lands, which
                        // is what stops a pinch turning into a stroke.
                        .pinchToZoom { factor, _ ->
                            zoom = (zoom * factor).coerceIn(MINIMUM_ZOOM, MAXIMUM_ZOOM)
                            if (zoom == 1f) pan = Offset.Zero
                        }
                        .twoFingerPanXY { delta -> pan += delta }
                        // Double-tap zooms about the tapped point, the same as the
                        // reader. A pinch needs two fingers and a deliberate spread;
                        // this is the one-handed way in, and it matters most here
                        // because the picture is what the whole screen is for.
                        //
                        // Safe alongside drawing: a tap that never moves is not a
                        // drag, so the canvas underneath does not claim it, and a
                        // stroke that *does* move never reaches the tap detector.
                        .pointerInput(Unit) {
                            detectTapGestures(
                                onDoubleTap = { position ->
                                    val target = if (zoom > MINIMUM_ZOOM + ZOOM_EPSILON) {
                                        MINIMUM_ZOOM
                                    } else {
                                        EDITOR_DOUBLE_TAP_ZOOM
                                    }
                                    // Keep whatever was under the finger under the
                                    // finger. Without this the picture jumps to its
                                    // centre and the detail being aimed at is gone.
                                    val ratio = target / zoom
                                    val centre = Offset(size.width / 2f, size.height / 2f)
                                    pan = if (target == MINIMUM_ZOOM) {
                                        Offset.Zero
                                    } else {
                                        (position - centre) * (1f - ratio) + pan * ratio
                                    }
                                    zoom = target
                                },
                            )
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    CaptureCanvas(
                        image = preview.preview,
                        crop = preview.request.localBounds,
                        markup = markup,
                        tool = markupTool,
                        color = markupColor,
                        size = markupSize,
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
                        size = markupSize,
                        onTool = onMarkupTool,
                        onColor = onMarkupColor,
                        onSize = onMarkupSize,
                        onPickCustomColour = { pickingColour = true },
                    )
    
                    // Scrollable rather than wrapped: on a phone in portrait
                    // five chips and the padding come to more than the screen
                    // is wide, and a chip that is simply off the edge is a
                    // setting nobody can reach.
                    Row(
                        modifier = Modifier.horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
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

                    // Only for a picture that has an outside. A box capture is all
                    // page, so a fill would be a control with nothing to act on.
                    if (preview.request.mask.isNotEmpty()) {
                        Row(
                            modifier = Modifier.horizontalScroll(rememberScrollState()),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                "Around it",
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            CaptureFill.entries.forEach { option ->
                                FilterChip(
                                    selected = fill == option,
                                    onClick = { onFillChange(option) },
                                    enabled = !isCapturing,
                                    label = { Text(option.label) },
                                )
                            }
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
 * What to draw with, how heavy, and in what colour.
 *
 * The pen is first and selected by default: it needs no aiming, and holding still
 * at the end of a stroke turns a rough circle or box into the tidy version anyway,
 * so the other tools are for when someone wants the shape without the hold.
 *
 * The sizes sit beside the colours and are always on screen, because "how thick"
 * is asked as often as "what colour" and burying it behind a press would make the
 * common case the slow one. The long press on a tool is for the size that is not
 * one of the four.
 */
@Composable
private fun MarkupTools(
    tool: MarkupTool,
    color: Long,
    size: Float,
    onTool: (MarkupTool) -> Unit,
    onColor: (Long) -> Unit,
    onSize: (MarkupTool, Float) -> Unit,
    onPickCustomColour: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(
            modifier = Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            MarkupToolButton(Icons.Filled.Draw, "Pen", MarkupTool.Pen, tool, size, onTool, onSize)
            MarkupToolButton(
                Icons.Filled.HorizontalRule,
                "Line",
                MarkupTool.Line,
                tool,
                size,
                onTool,
                onSize,
            )
            MarkupToolButton(
                Icons.AutoMirrored.Filled.ArrowRightAlt,
                "Arrow",
                MarkupTool.Arrow,
                tool,
                size,
                onTool,
                onSize,
            )
            MarkupToolButton(
                Icons.Filled.CheckBoxOutlineBlank,
                "Box",
                MarkupTool.Rectangle,
                tool,
                size,
                onTool,
                onSize,
            )
            MarkupToolButton(
                Icons.Filled.RadioButtonUnchecked,
                "Circle",
                MarkupTool.Ellipse,
                tool,
                size,
                onTool,
                onSize,
            )
            MarkupToolButton(
                Icons.Filled.Highlight,
                "Highlight",
                MarkupTool.Highlight,
                tool,
                size,
                onTool,
                onSize,
            )
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SizeButton(tool = tool, size = size, color = color, onSize = onSize)

            Row(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // The wheel first, before the palette. It is the way to *any*
                // colour, so it belongs where the eye starts rather than tucked
                // behind six that happened to be chosen in advance.
                CustomColourSwatch(
                    current = color,
                    isCustom = color !in MARKUP_COLOURS,
                    onClick = onPickCustomColour,
                )
                MARKUP_COLOURS.forEach { swatch ->
                    ColourSwatch(
                        colour = swatch,
                        selected = swatch == color,
                        onClick = { onColor(swatch) },
                    )
                }
            }
        }
    }
}

/**
 * One control for how heavy the tool draws: the current size, and the rest behind
 * a press.
 *
 * Four dots in a row said everything at once and cost four slots of a toolbar
 * that has to fit a phone in portrait. One dot says the thing that is true now —
 * drawn at the size it means, so it answers "how will this look" rather than
 * "which number is selected" — and a press offers the others.
 */
@Composable
private fun SizeButton(
    tool: MarkupTool,
    size: Float,
    color: Long,
    onSize: (MarkupTool, Float) -> Unit,
) {
    var choosing by remember { mutableStateOf(false) }

    Box {
        Box(
            modifier = Modifier
                .size(36.dp)
                .background(MaterialTheme.colorScheme.secondaryContainer, CircleShape)
                .combinedClickableCompat(
                    // Tap opens it too. A control that only answers to a long
                    // press is one most people never discover, and this one has to
                    // be reachable — it is the only way to change the size at all.
                    onClick = { choosing = true },
                    onLongClick = { choosing = true },
                ),
            contentAlignment = Alignment.Center,
        ) {
            SizeMark(tool = tool, size = size, color = color, diameter = 24.dp)
        }

        if (choosing) {
            SizeChooser(
                tool = tool,
                size = size,
                color = color,
                onSize = { onSize(tool, it) },
                onDismiss = { choosing = false },
            )
        }
    }
}

/**
 * What a size looks like: a dot at the nib's width, or a bar at the wash's
 * strength.
 *
 * Shared by the button and the choices inside it, so the one on the toolbar is
 * literally the one that was picked.
 */
@Composable
private fun SizeMark(tool: MarkupTool, size: Float, color: Long, diameter: Dp) {
    Canvas(Modifier.size(diameter)) {
        if (tool.isIntensity) {
            drawRect(
                color = Color(color).copy(alpha = size),
                topLeft = Offset(0f, this.size.height * 0.25f),
                size = Size(this.size.width, this.size.height * 0.5f),
            )
        } else {
            // Scaled rather than true to size: a 16-unit nib drawn actual-size
            // would fill the slot and a 0.6 one would be invisible. The order is
            // what carries the meaning, and the clamps keep both ends usable.
            drawCircle(
                color = Color(color),
                radius = (size * PRESET_DOT_SCALE).coerceIn(2f, this.size.minDimension / 2f),
            )
        }
    }
}

/**
 * The sizes on offer, and a slider for the ones that are not.
 *
 * Both in one popup because they answer the same question at different
 * precisions: the presets are the common answers, the slider is for when none of
 * them is quite right.
 */
@Composable
private fun SizeChooser(
    tool: MarkupTool,
    size: Float,
    color: Long,
    onSize: (Float) -> Unit,
    onDismiss: () -> Unit,
) {
    Popup(
        alignment = Alignment.TopCenter,
        offset = IntOffset(0, -POPUP_LIFT_PX),
        onDismissRequest = onDismiss,
        properties = PopupProperties(focusable = true),
    ) {
        Surface(
            shape = RoundedCornerShape(16.dp),
            tonalElevation = 3.dp,
            shadowElevation = 8.dp,
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            Column(
                modifier = Modifier.width(260.dp).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    if (tool.isIntensity) {
                        "Intensity · ${(size * 100).roundToInt()}%"
                    } else {
                        "Thickness · ${"%.1f".format(size)}"
                    },
                    style = MaterialTheme.typography.labelLarge,
                )

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    tool.sizePresets.forEach { preset ->
                        val chosen = kotlin.math.abs(preset - size) < 0.01f
                        Box(
                            modifier = Modifier
                                .size(44.dp)
                                .background(
                                    color = if (chosen) {
                                        MaterialTheme.colorScheme.secondaryContainer
                                    } else {
                                        Color.Transparent
                                    },
                                    shape = CircleShape,
                                )
                                .clickable { onSize(preset) },
                            contentAlignment = Alignment.Center,
                        ) {
                            SizeMark(tool = tool, size = preset, color = color, diameter = 30.dp)
                        }
                    }
                }

                Slider(value = size, onValueChange = onSize, valueRange = tool.sizeRange)
            }
        }
    }
}

@Composable
private fun ColourSwatch(colour: Long, selected: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(26.dp)
            .background(Color(colour), CircleShape)
            .border(
                width = if (selected) 3.dp else 1.dp,
                color = if (selected) {
                    MaterialTheme.colorScheme.onSurface
                } else {
                    MaterialTheme.colorScheme.outlineVariant
                },
                shape = CircleShape,
            )
            .clickable(onClick = onClick),
    )
}

/**
 * The way out of the palette.
 *
 * Shows the wheel it opens until a colour has been picked from it, and the colour
 * itself afterwards — otherwise choosing a custom colour makes the selection ring
 * vanish from the row and nothing on screen says what is being drawn with.
 */
@Composable
private fun CustomColourSwatch(current: Long, isCustom: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(26.dp)
            .border(
                width = if (isCustom) 3.dp else 1.dp,
                color = if (isCustom) {
                    MaterialTheme.colorScheme.onSurface
                } else {
                    MaterialTheme.colorScheme.outlineVariant
                },
                shape = CircleShape,
            )
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(22.dp)) {
            if (isCustom) {
                drawCircle(Color(current))
            } else {
                drawCircle(
                    brush = Brush.sweepGradient(
                        listOf(
                            Color.Red,
                            Color.Yellow,
                            Color.Green,
                            Color.Cyan,
                            Color.Blue,
                            Color.Magenta,
                            Color.Red,
                        ),
                    ),
                )
            }
        }
    }
}

/**
 * A tool, with its size a long press away.
 *
 * Long press selects the tool as well as opening the slider. Opening a size
 * control for a tool you are not using would be a strange thing to offer, and
 * having to tap first and then press again is a step for nothing.
 */
@Composable
private fun MarkupToolButton(
    icon: ImageVector,
    label: String,
    represents: MarkupTool,
    selected: MarkupTool,
    size: Float,
    onTool: (MarkupTool) -> Unit,
    onSize: (MarkupTool, Float) -> Unit,
) {
    val isSelected = represents == selected
    var adjusting by remember { mutableStateOf(false) }

    Box {
        Box(
            modifier = Modifier
                .size(44.dp)
                .background(
                    color = if (isSelected) {
                        MaterialTheme.colorScheme.secondaryContainer
                    } else {
                        Color.Transparent
                    },
                    shape = CircleShape,
                )
                .combinedClickableCompat(
                    onClick = { onTool(represents) },
                    onLongClick = {
                        onTool(represents)
                        adjusting = true
                    },
                ),
            contentAlignment = Alignment.Center,
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

        if (adjusting) {
            SizePopup(
                tool = represents,
                label = label,
                size = size,
                onSize = { onSize(represents, it) },
                onDismiss = { adjusting = false },
            )
        }
    }
}

/**
 * The slider, over the tool it belongs to.
 *
 * A popup rather than a dialog: it is a small adjustment to something visible,
 * and a dialog would take the whole screen away from the picture being marked up.
 */
@Composable
private fun SizePopup(
    tool: MarkupTool,
    label: String,
    size: Float,
    onSize: (Float) -> Unit,
    onDismiss: () -> Unit,
) {
    Popup(
        alignment = Alignment.TopCenter,
        // Above the button, which is near the bottom of the screen.
        offset = IntOffset(0, -POPUP_LIFT_PX),
        onDismissRequest = onDismiss,
        properties = PopupProperties(focusable = true),
    ) {
        Surface(
            shape = RoundedCornerShape(16.dp),
            tonalElevation = 3.dp,
            shadowElevation = 8.dp,
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            Column(
                modifier = Modifier.width(240.dp).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    if (tool.isIntensity) {
                        "$label · ${(size * 100).roundToInt()}%"
                    } else {
                        "$label · ${"%.1f".format(size)}"
                    },
                    style = MaterialTheme.typography.labelLarge,
                )
                Slider(
                    value = size,
                    onValueChange = onSize,
                    valueRange = tool.sizeRange,
                )
            }
        }
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

/** Where a double-tap lands. The reader's figure, so the two feel like one app. */
private const val EDITOR_DOUBLE_TAP_ZOOM = 2.5f

/** Float slack for "is it zoomed": a pinch rarely leaves the scale at exactly 1. */
private const val ZOOM_EPSILON = 0.01f

/**
 * How far a preset dot's radius is from the nib width it stands for.
 *
 * The dots are a rank, not a ruler: a 16-unit nib drawn at true size would fill
 * its slot and a 0.6 one would be invisible, so the scale keeps the order and the
 * clamps keep both ends usable.
 */
private const val PRESET_DOT_SCALE = 2.2f

/** How far above the tool row the size popup sits, in pixels. */
private const val POPUP_LIFT_PX = 240

/**
 * The backdrop that says "nothing here".
 *
 * The same two-tone grid every image editor uses, for the same reason: it is the
 * one pattern nobody mistakes for part of the picture.
 */
private fun DrawScope.drawCheckerboard() {
    val step = CHECKER_SQUARE_PX
    val light = Color(0xFF3A3A3E)
    val dark = Color(0xFF2C2C30)
    drawRect(light)

    var row = 0
    var y = 0f
    while (y < size.height) {
        var column = 0
        var x = 0f
        while (x < size.width) {
            if ((row + column) % 2 == 0) {
                drawRect(
                    color = dark,
                    topLeft = Offset(x, y),
                    size = Size(
                        minOf(step, size.width - x),
                        minOf(step, size.height - y),
                    ),
                )
            }
            x += step
            column++
        }
        y += step
        row++
    }
}

/** Checkerboard square, in pixels. Big enough to read, small enough to recede. */
private const val CHECKER_SQUARE_PX = 24f
