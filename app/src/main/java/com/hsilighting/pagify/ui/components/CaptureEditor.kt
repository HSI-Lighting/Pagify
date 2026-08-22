package com.hsilighting.pagify.ui.components

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.Markup
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.bendsText
import com.hsilighting.pagify.core.curvedBaseline
import com.hsilighting.pagify.core.straightBaseline
import com.hsilighting.pagify.core.textFrame
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import com.hsilighting.pagify.core.MarkupShape
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.RING_MARKUP_TOOLS
import com.hsilighting.pagify.core.hasLineStyle
import com.hsilighting.pagify.core.isBroken
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.isIntensity
import com.hsilighting.pagify.core.sizePresets
import com.hsilighting.pagify.core.sizeRange
import com.hsilighting.pagify.ui.reader.CapturePreview
import kotlin.math.abs
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
    /** Whether that tool is actually held; see `PdfReaderState.markupArmed`. */
    markupArmed: Boolean,
    onDisarmMarkup: () -> Unit,
    markupColor: Long,
    /** How heavy the current tool draws: nib width, or intensity for the wash. */
    markupSize: Float,
    /** Solid, dashed or dash-dot, for the line tool. */
    markupStyle: MarkupStyle,
    onMarkupStyle: (MarkupStyle) -> Unit,
    onScaleChange: (CaptureScale) -> Unit,
    onFormatChange: (CaptureFormat) -> Unit,
    /** What fills the picture where no page reaches. */
    fill: CaptureFill,
    onFillChange: (CaptureFill) -> Unit,
    onMarkupTool: (MarkupTool) -> Unit,
    onMarkupColor: (Long) -> Unit,
    onMarkupSize: (MarkupTool, Float) -> Unit,
    /** The face words are written in, shared with the reader's text tools. */
    textFont: PdfFont,
    textSizePoints: Float,
    textCurveDegrees: Float,
    onTextFont: (PdfFont) -> Unit,
    onTextSize: (Float) -> Unit,
    onTextCurve: (Float) -> Unit,
    onCommitMarkup: (MarkupShape) -> Unit,
    onRecogniseMarkup: (List<Offset>) -> Unit,
    onUndoMarkup: () -> Unit,
    /** Words already on the picture have been dragged; move that mark. */
    onMoveMarkup: (index: Int, delta: Offset) -> Unit,
    /** A caption on the picture was tapped; the ribbon's controls now edit it. */
    onSelectMarkup: (index: Int) -> Unit,
    /** Two fingers with a caption in hand: that big. */
    onScaleMarkup: (factor: Float) -> Unit,
    /** Which caption the ribbon is editing, if one is picked up. */
    selectedMarkup: Int?,
    onSaveToGallery: () -> Unit,
    onShare: () -> Unit,
    onCopy: () -> Unit,
    onDismiss: () -> Unit,
) {
    var zoom by remember { mutableFloatStateOf(1f) }
    var pan by remember { mutableStateOf(Offset.Zero) }

    /**
     * The baseline waiting for its words.
     *
     * Kept here rather than in the reader's state: it lives and dies with this
     * sheet, and a half-typed caption has no meaning once the picture is gone.
     */
    var pendingText by remember { mutableStateOf<List<Offset>?>(null) }

    /** The picture area's size, for zooming about a point rather than the middle. */
    var viewport by remember { mutableStateOf(IntSize.Zero) }
    var pickingColour by remember { mutableStateOf(false) }
    var typed by remember { mutableStateOf("") }

    /** Which export is being set up, if one is. */
    var exporting by remember { mutableStateOf<ExportAction?>(null) }

    exporting?.let { action ->
        ExportSheet(
            action = action,
            scale = preview.request.scale,
            format = preview.request.format,
            fill = fill,
            // A box capture is all page, so there is nothing around it to fill and
            // the question is not asked. A drawn-around one always has an outside.
            hasBareArea = preview.request.mask.isNotEmpty(),
            isCapturing = isCapturing,
            onScale = onScaleChange,
            onFormat = onFormatChange,
            onFill = onFillChange,
            onDismiss = { exporting = null },
            onConfirm = {
                exporting = null
                when (action) {
                    ExportAction.Save -> onSaveToGallery()
                    ExportAction.Share -> onShare()
                    ExportAction.Copy -> onCopy()
                }
            },
        )
    }

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

        pendingText?.let { baseline ->
            AlertDialog(
                onDismissRequest = {
                    pendingText = null
                    typed = ""
                },
                title = { Text("Text") },
                text = {
                    OutlinedTextField(
                        value = typed,
                        onValueChange = { typed = it },
                        singleLine = false,
                        placeholder = { Text("Type here") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                confirmButton = {
                    TextButton(
                        enabled = typed.isNotBlank(),
                        onClick = {
                            // Blank words make no mark rather than an invisible
                            // one: nothing to see, and nothing to rub out except
                            // by guessing where it was.
                            onCommitMarkup(
                                MarkupShape.Text(
                                    text = typed,
                                    // A tap gives only the point it landed on,
                                    // and the layout walks a line: the baseline
                                    // has to be as long as the words, which is
                                    // not known until the words exist.
                                    // A tap gives one point and the layout walks
                                    // a line; the bend is a setting, so the line
                                    // is built rather than traced.
                                    path = curvedBaseline(
                                        anchor = baseline.first(),
                                        text = typed,
                                        font = textFont,
                                        sizePoints = textSizePoints,
                                        degrees = if (markupTool.bendsText) {
                                            textCurveDegrees
                                        } else {
                                            0f
                                        },
                                    ),
                                    font = textFont,
                                    sizePoints = textSizePoints,
                                    frame = markupTool.textFrame,
                                ),
                            )
                            pendingText = null
                            typed = ""
                        },
                    ) { Text("Add") }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            pendingText = null
                            typed = ""
                        },
                    ) { Text("Cancel") }
                },
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
                        // A second guard, on the area rather than the picture:
                        // whatever the canvas does, nothing inside here reaches
                        // the controls below it.
                        .clipToBounds()
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
                            // A caption in hand takes the pinch, as in the reader:
                            // while one is held two fingers mean "this big", and
                            // the picture holds still. Tapping empty picture puts
                            // it down and gives the zoom back.
                            if (selectedMarkup != null) {
                                onScaleMarkup(factor)
                                return@pinchToZoom
                            }
                            zoom = (zoom * factor).coerceIn(MINIMUM_ZOOM, MAXIMUM_ZOOM)
                            if (zoom == 1f) pan = Offset.Zero
                        }
                        // The pan half of the same gesture. Gating only the zoom
                        // left the picture sliding about under a caption being
                        // resized — the two handlers read the same two fingers, so
                        // both have to stand down, not one.
                        .twoFingerPanXY { delta ->
                            if (selectedMarkup == null) pan += delta
                        }
                        // Double-tap zooms about the tapped point, the same as the
                        // reader. A pinch needs two fingers and a deliberate spread;
                        // this is the one-handed way in, and it matters most here
                        // because the picture is what the whole screen is for.
                        //
                        // Watched on the Initial pass, not detected on the Main
                        // one — the same trap the reader documents. The canvas
                        // underneath claims the press the moment a tool is armed,
                        // and `detectTapGestures` asks for one nobody has claimed,
                        // so with anything in hand the zoom never saw a tap at all.
                        .onSizeChanged { viewport = it }
                        .doubleTapToZoom { position ->
                            val target = if (zoom > MINIMUM_ZOOM + ZOOM_EPSILON) {
                                MINIMUM_ZOOM
                            } else {
                                EDITOR_DOUBLE_TAP_ZOOM
                            }
                            // Keep whatever was under the finger under the finger.
                            // Without this the picture jumps to its centre and the
                            // detail being aimed at is gone.
                            val ratio = target / zoom
                            val centre = Offset(viewport.width / 2f, viewport.height / 2f)
                            pan = if (target == MINIMUM_ZOOM) {
                                Offset.Zero
                            } else {
                                (position - centre) * (1f - ratio) + pan * ratio
                            }
                            zoom = target
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    CaptureCanvas(
                        image = preview.preview,
                        crop = preview.request.localBounds,
                        markup = markup,
                        tool = markupTool,
                        armed = markupArmed,
                        color = markupColor,
                        size = markupSize,
                        style = markupStyle,
                        onCommit = onCommitMarkup,
                        onRecognise = onRecogniseMarkup,
                        onPlaceText = { pendingText = it },
                        onMoveText = onMoveMarkup,
                        onSelectText = onSelectMarkup,
                        selectedText = selectedMarkup,
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
                    MarkupRibbon(
                        tool = markupTool,
                        armed = markupArmed,
                        onDisarm = onDisarmMarkup,
                        color = markupColor,
                        size = markupSize,
                        style = markupStyle,
                        font = textFont,
                        sizePoints = textSizePoints,
                        onTool = onMarkupTool,
                        onColor = onMarkupColor,
                        onSize = onMarkupSize,
                        onStyle = onMarkupStyle,
                        curveDegrees = textCurveDegrees,
                        onFont = onTextFont,
                        onCurve = onTextCurve,
                        onSizePoints = onTextSize,
                        onPickCustomColour = { pickingColour = true },
                    )

                    // How sharp and what kind of file used to sit here, on screen
                    // the whole time the picture was being marked up. They are not
                    // markup: they are questions about the *file*, and the moment
                    // to ask them is the moment there is going to be one. Asking
                    // then also means the answers can be remembered, so the common
                    // case is a sheet that is already right.
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Button(
                            onClick = { exporting = ExportAction.Save },
                            enabled = !isCapturing,
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(Icons.Filled.Download, contentDescription = null, Modifier.size(18.dp))
                            Text("  Save")
                        }
                        OutlinedButton(
                            onClick = { exporting = ExportAction.Share },
                            enabled = !isCapturing,
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(Icons.Filled.Share, contentDescription = null, Modifier.size(18.dp))
                            Text("  Share")
                        }
                        OutlinedButton(
                            onClick = { exporting = ExportAction.Copy },
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
 * Ink colours.
 *
 * Red first, and the default: markup on a page is almost always pointing
 * something out, and it has to hold up next to black text.
 */
internal val MARKUP_COLOURS = listOf(
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

/**
 * Which line type the next mark is drawn in.
 *
 * The glyph *is* the setting: a line in the pattern it selects, rather than a
 * label or an abstract icon. "Centerline-2" means nothing until you have seen
 * one, and five names would not fit the slot in any case.
 *
 * A tap opens the list rather than stepping to the next type. Cycling was fine
 * when there were two; with five, reaching the last one is four taps and a lot of
 * squinting at a 26dp glyph.
 *
 * It applies to every tool that draws a line — the pen, the line, the arrow, the
 * box, the circle — so unlike the tools beside it, this one does not change what
 * you are drawing with. It sits lit only while a broken type is live.
 */
@Composable
private fun LineStyleButton(
    style: MarkupStyle,
    active: Boolean,
    color: Long,
    onStyle: (MarkupStyle) -> Unit,
) {
    var choosing by remember { mutableStateOf(false) }

    Box {
        Box(
            modifier = Modifier
                .size(44.dp)
                .background(
                    color = if (active) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        Color.Transparent
                    },
                    shape = CircleShape,
                )
                .combinedClickableCompat(
                    onClick = { choosing = true },
                    onLongClick = { choosing = true },
                )
                .longPressHint(
                    tint = if (active) {
                        MaterialTheme.colorScheme.onPrimary
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                ),
            contentAlignment = Alignment.Center,
        ) {
            StylePattern(
                style = style,
                tint = if (active) {
                    MaterialTheme.colorScheme.onPrimary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                width = 26.dp,
            )
        }

        DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
            MarkupStyle.entries.forEach { option ->
                DropdownMenuItem(
                    text = { Text(option.label) },
                    leadingIcon = {
                        StylePattern(
                            style = option,
                            tint = if (option == style) {
                                MaterialTheme.colorScheme.primary
                            } else {
                                MaterialTheme.colorScheme.onSurface
                            },
                            width = 36.dp,
                        )
                    },
                    onClick = {
                        choosing = false
                        onStyle(option)
                    },
                )
            }
        }
    }
}

/**
 * A short line in the pattern it names.
 *
 * Drawn rather than three glyphs, because the dash lengths have to match what the
 * engine will actually draw — a picture of a dashed line that dashes differently
 * from the export is worse than no picture.
 */
@Composable
private fun StylePattern(style: MarkupStyle, tint: Color, width: Dp) {
    Canvas(Modifier.size(width, 12.dp)) {
        val thickness = STYLE_PATTERN_WIDTH_PX
        val effect = style.pathEffect(thickness)
        drawLine(
            color = tint,
            start = Offset(0f, size.height / 2f),
            end = Offset(size.width, size.height / 2f),
            strokeWidth = thickness,
            cap = StrokeCap.Round,
            pathEffect = effect,
        )
    }
}

/** How thick the little pattern is drawn, in pixels. */
private const val STYLE_PATTERN_WIDTH_PX = 4f
