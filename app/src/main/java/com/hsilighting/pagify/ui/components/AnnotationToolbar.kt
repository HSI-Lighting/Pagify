package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.filled.Brush
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.HorizontalRule
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.filled.CropFree
import androidx.compose.material.icons.filled.Draw
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.filled.Gesture
import androidx.compose.material.icons.filled.HistoryEdu
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.DRAWING_TOOLS
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.draws
import com.hsilighting.pagify.core.marks
import kotlin.math.roundToInt

/**
 * The floating tool ribbon along the bottom of the reader.
 *
 * Tapping a tool selects it, and tapping the selected tool again puts the reader
 * back to plain scrolling — a tool that can only be turned on is a trap, since
 * every touch would keep drawing.
 *
 * The drawing tools share one slot. Pen, line, arrow, box, circle and cloud would
 * be six slots on a ribbon that already has seven, and they are one question
 * anyway: what shape is this mark. The armed one is what the slot shows.
 *
 * Two gestures, and the same two throughout: **a tap chooses, a press adjusts.**
 * Tapping the drawing slot opens the shapes; pressing it opens what they draw
 * *with* — the size, the colour, the line type. Pressing the highlighter does the
 * same for its colours. Choosing the shape is the frequent act and the settings
 * are the occasional one, so the frequent one is the tap.
 *
 * The one press that goes deeper is on the circle, inside the shapes: it offers
 * the cloud, which is the mark rare enough to sit a level down.
 */
@Composable
fun AnnotationToolbar(
    selectedTool: AnnotationTool,
    penColor: Long,
    /** How heavy the drawing tools are, in page points, and what line they draw. */
    strokeWidth: Float,
    lineStyle: MarkupStyle,
    /** What text is written in, and how big. */
    textFont: PdfFont,
    textSizePoints: Float,
    textCurveDegrees: Float,
    /** The largest size a caption can take and still fit the page. */
    textSizeCeiling: Float,
    /** Whether the bend still means anything for the caption in hand. */
    textBendApplies: Boolean,
    onTextFont: (PdfFont) -> Unit,
    /** How far a curved caption bends, end to end, in degrees. */
    onTextCurve: (Float) -> Unit,
    onTextSize: (Float) -> Unit,
    onSelectTool: (AnnotationTool) -> Unit,
    onPenColorChange: (Long) -> Unit,
    onStrokeWidth: (Float) -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    /** Marks on the page being read, and in the document as a whole. */
    marksOnPage: Int,
    marksInDocument: Int,
    onClearPage: () -> Unit,
    onClearAll: () -> Unit,
    /** Whether the capture tool draws a ring instead of dragging a box. */
    captureLasso: Boolean,
    onCaptureLasso: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    var showDrawPalette by remember { mutableStateOf(false) }
    var showClearMenu by remember { mutableStateOf(false) }


    /**
     * Whether the band of settings is showing.
     *
     * Asked for rather than automatic. It used to appear the moment anything was
     * armed, which put a row of controls over the page every time a tool was
     * picked up — for a mark that usually wants the same colour and weight as the
     * last one.
     */
    var showParameters by remember { mutableStateOf(false) }

    /** Whether the colour wheel is open, for a colour the six do not offer. */
    var pickingColour by remember { mutableStateOf(false) }

    if (pickingColour) {
        ColourWheelDialog(
            initial = penColor,
            onPick = {
                onPenColorChange(it)
                pickingColour = false
            },
            onDismiss = { pickingColour = false },
        )
    }

    /**
     * Arm a tool, and put every palette away.
     *
     * A palette is a way of *choosing*; once something is chosen it has nothing
     * left to say. Closing only on its own selection is what left the pen's five
     * shapes hanging above the highlighter's colours — two bands offering two
     * different tools, one of them already dismissed in every sense but the
     * visible one. Routed through here rather than fixed per button, so the next
     * tool added to the ribbon cannot forget it.
     */
    fun select(tool: AnnotationTool) {
        showDrawPalette = false
        showClearMenu = false
        showParameters = false
        onSelectTool(tool)
    }

    /**
     * Show what a tool draws with, arming it if it is not already.
     *
     * Arming it is the point: the width, the colour and the line type are one set
     * shared by every tool that draws, so opening them for a tool you are not
     * holding would change the next mark rather than this one.
     */
    fun openParameters(tool: AnnotationTool) {
        showDrawPalette = false
        showClearMenu = false
        if (selectedTool != tool) onSelectTool(tool)
        showParameters = true
    }

    // Everything stacked in one column rather than floated at a fixed height
    // above the ribbon: with the parameters band there too, a palette lifted by a
    // constant lands on top of it — and being drawn first, underneath it.
    Box(modifier, contentAlignment = Alignment.BottomCenter) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (showDrawPalette) {
                DrawingRibbon(
                    selectedTool = selectedTool,
                    color = penColor,
                    strokeWidth = strokeWidth,
                    lineStyle = lineStyle,
                    font = textFont,
                    sizePoints = textSizePoints,
                    curveDegrees = textCurveDegrees,
                    sizeCeiling = textSizeCeiling,
                    bendApplies = textBendApplies,
                    onFont = onTextFont,
                    onCurve = onTextCurve,
                    onSizePoints = onTextSize,
                    // The row stays open when a tool is chosen from it. It is a
                    // workspace, not a menu: the next thing after picking a shape
                    // is usually setting the colour or the weight for it.
                    onSelectTool = onSelectTool,
                    onColor = onPenColorChange,
                    onStrokeWidth = onStrokeWidth,
                    onLineStyle = onLineStyle,
                    onPickCustomColour = { pickingColour = true },
                    onDismiss = { showDrawPalette = false },
                )
            }
            if (showClearMenu) {
                ClearMenu(
                    marksOnPage = marksOnPage,
                    marksInDocument = marksInDocument,
                    onClearPage = onClearPage,
                    onClearAll = onClearAll,
                    onDismiss = { showClearMenu = false },
                )
            }

            // The highlighter's colours. The drawing tools carry theirs in the
            // drawing ribbon above; the highlighter is not part of that row, and
            // one colour is all it has.
            if (showParameters && selectedTool == AnnotationTool.Highlight) {
                MarkParameters(
                    tool = selectedTool,
                    color = penColor,
                    strokeWidth = strokeWidth,
                    lineStyle = lineStyle,
                    onColor = onPenColorChange,
                    onStrokeWidth = onStrokeWidth,
                    onLineStyle = onLineStyle,
                    onPickCustomColour = { pickingColour = true },
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
                        icon = Icons.Filled.Draw,
                        label = "Highlighter",
                        selected = selectedTool == AnnotationTool.Highlight,
                        accent = Color(penColor),
                        onClick = {
                            select(toggle(selectedTool, AnnotationTool.Highlight))
                        },
                        onLongClick = { openParameters(AnnotationTool.Highlight) },
                        hasMore = true,
                    )
                    ToolButton(
                        // The armed tool's own glyph, so the ribbon says what a
                        // drag will draw rather than naming the group.
                        icon = drawingToolIcon(selectedTool),
                        label = drawingToolLabel(selectedTool),
                        selected = selectedTool.draws,
                        accent = Color(penColor),
                        // Tap opens the tools; the press opens what they draw
                        // with. That way round because choosing the shape is the
                        // frequent thing and the settings are the occasional one,
                        // and because a band that appeared by itself the moment a
                        // tool was armed took a third of the page uninvited.
                        // Tapping the group while one of its tools is armed puts
                        // the pen down. Going into the palette to tap the exact
                        // tool again was the only way to stop drawing, which is a
                        // lot of aim for "I am finished".
                        onClick = {
                            showParameters = false
                            if (selectedTool.draws) {
                                showDrawPalette = false
                                onSelectTool(AnnotationTool.None)
                            } else {
                                showDrawPalette = !showDrawPalette
                            }
                        },
                        onLongClick = {
                            openParameters(
                                if (selectedTool.draws) selectedTool else AnnotationTool.Pen,
                            )
                        },
                        hasMore = true,
                    )
                    ToolButton(
                        icon = Icons.Filled.TextFields,
                        label = "Note",
                        selected = selectedTool == AnnotationTool.Note,
                        onClick = { select(toggle(selectedTool, AnnotationTool.Note)) },
                    )
                    ToolButton(
                        // Distinct from the pen on purpose: both were Draw, and two
                        // identical glyphs in a four-slot ribbon is unreadable.
                        icon = Icons.Filled.HistoryEdu,
                        label = "Signature",
                        selected = selectedTool == AnnotationTool.Signature,
                        onClick = { select(toggle(selectedTool, AnnotationTool.Signature)) },
                    )
                    ToolButton(
                        icon = EraserIcon,
                        label = "Eraser",
                        selected = selectedTool == AnnotationTool.Eraser,
                        onClick = { select(toggle(selectedTool, AnnotationTool.Eraser)) },
                        onLongClick = {
                            // Clearing a page or the document lives behind the eraser
                            // because that is what it means — the same action, wider.
                            // Long press does not select the tool, so a wipe is not
                            // followed by an armed eraser under your finger.
                            showClearMenu = true
                        },
                        hasMore = true,
                    )
                    ToolButton(
                        icon = Icons.Filled.CropFree,
                        label = "Snapshot",
                        selected = selectedTool == AnnotationTool.Snapshot && !captureLasso,
                        onClick = {
                            onCaptureLasso(false)
                            select(
                                if (selectedTool == AnnotationTool.Snapshot && !captureLasso) {
                                    AnnotationTool.None
                                } else {
                                    AnnotationTool.Snapshot
                                },
                            )
                        },
                    )
                    ToolButton(
                        // Its own slot rather than a shape hidden behind a long press on
                        // the one beside it. They are two tools by the time you are
                        // choosing: a box for most things, a ring for the detail a box
                        // cannot take without its neighbours.
                        icon = Icons.Filled.Gesture,
                        label = "Draw around",
                        selected = selectedTool == AnnotationTool.Snapshot && captureLasso,
                        onClick = {
                            onCaptureLasso(true)
                            select(
                                if (selectedTool == AnnotationTool.Snapshot && captureLasso) {
                                    AnnotationTool.None
                                } else {
                                    AnnotationTool.Snapshot
                                },
                            )
                        },
                    )
                }
            }
        }
    }
}

private fun toggle(current: AnnotationTool, tapped: AnnotationTool): AnnotationTool =
    if (current == tapped) AnnotationTool.None else tapped

@Composable
internal fun ToolButton(
    icon: ImageVector,
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    accent: Color? = null,
    /** Draws the wedge that says a long press offers more. */
    hasMore: Boolean = false,
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
            )
            .then(
                if (hasMore) {
                    Modifier.longPressHint(
                        tint = if (selected) {
                            MaterialTheme.colorScheme.onPrimary
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                    )
                } else {
                    Modifier
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
/**
 * The colours the ribbon offers.
 *
 * Ink rather than highlighter: these are for lines drawn on a page, and a wash
 * yellow that reads well behind text is nearly invisible as a line on white.
 */
private val DRAWING_COLOURS = AnnotationColors.markerPalette

@Composable
internal fun ColourDot(colour: Long, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .size(30.dp)
            .clip(CircleShape)
            .background(Color(colour))
            .border(
                width = if (selected) 3.dp else 1.dp,
                color = if (selected) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.outlineVariant
                },
                shape = CircleShape,
            )
            .clickable(onClick = onClick),
    )
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

/**
 * What to do with the snapshot tool armed.
 *
 * Every other tool announces itself the moment the page is touched — a stroke
 * appears, a marker lands, something is rubbed out. This one does nothing at all
 * until a rectangle has been dragged, so with nothing on screen an armed snapshot
 * tool is indistinguishable from a tool that does not work.
 */
@Composable
fun CaptureHint(lasso: Boolean, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Text(
            text = if (lasso) {
                "Draw around what you want to keep"
            } else {
                "Drag a box around what you want to keep"
            },
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
    }
}

/**
 * Typing the text of a new note.
 *
 * A dialog rather than an inline field on the page: the keyboard covers roughly
 * half a tablet screen, and an anchored editor would sit under it exactly when
 * the note is near the bottom of a page — which is where the reader has just
 * scrolled to in order to tap there.
 */
@Composable
fun NoteComposer(onConfirm: (String) -> Unit, onDismiss: () -> Unit) {
    var text by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }

    // The dialog exists to be typed into, so it asks for the keyboard itself
    // rather than making the reader tap the field they just asked for.
    LaunchedEffect(Unit) { focus.requestFocus() }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a note") },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                placeholder = { Text("Note") },
                modifier = Modifier.focusRequester(focus),
            )
        },
        confirmButton = {
            // Disabled on blank: an empty note draws as a marker with nothing in
            // it, which cannot be told apart from a stray tap.
            TextButton(onClick = { onConfirm(text) }, enabled = text.isNotBlank()) {
                Text("Add")
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * Reading a note that is already on the page.
 *
 * The counterpart to [NoteComposer], and the half that was missing: a note's text
 * was stored and then unreachable, so the tool appeared to add a marker and
 * nothing else.
 */
@Composable
fun NoteReader(text: String, onDelete: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Note") },
        text = { Text(text.ifBlank { "(empty)" }) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
        dismissButton = { TextButton(onClick = onDelete) { Text("Delete") } },
    )
}


/** The glyph for whichever drawing tool is armed, or the pen when none is. */
private fun drawingToolIcon(tool: AnnotationTool): ImageVector = when (tool) {
    AnnotationTool.Line -> Icons.Filled.HorizontalRule
    AnnotationTool.Arrow -> Icons.AutoMirrored.Filled.ArrowRightAlt
    AnnotationTool.Rectangle -> Icons.Filled.CheckBoxOutlineBlank
    AnnotationTool.Ellipse -> Icons.Filled.RadioButtonUnchecked
    AnnotationTool.Cloud -> Icons.Outlined.Cloud
    else -> Icons.Filled.Brush
}

private fun drawingToolLabel(tool: AnnotationTool): String = when (tool) {
    AnnotationTool.Line -> "Line"
    AnnotationTool.Arrow -> "Arrow"
    AnnotationTool.Rectangle -> "Box"
    AnnotationTool.Ellipse -> "Circle"
    AnnotationTool.Cloud -> "Cloud"
    else -> "Pen"
}


/**
 * What the armed tool draws with: how heavy, in what colour, as what kind of line.
 *
 * A band of its own above the ribbon rather than a palette behind a press. These
 * are the settings that change between one mark and the next — a thick red circle
 * around a fault, then a fine dashed line to where it goes — and a setting changed
 * that often should not cost a long press each time.
 *
 * The line type is offered for every drawing tool, the pen included: a dashed
 * freehand line is a legitimate mark, and the tool that drew it makes no
 * difference to what a dash means.
 */
@Composable
private fun MarkParameters(
    tool: AnnotationTool,
    color: Long,
    strokeWidth: Float,
    lineStyle: MarkupStyle,
    onColor: (Long) -> Unit,
    onStrokeWidth: (Float) -> Unit,
    /** Opens the wheel, for a colour none of the six offers. */
    onPickCustomColour: () -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 6.dp,
    ) {
        Column(
            modifier = Modifier
                .padding(horizontal = 10.dp, vertical = 8.dp)
                .widthIn(max = PARAMETER_BAND_WIDTH),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // The highlighter has a colour and nothing else: no nib width, no
            // line type. Showing it controls that do nothing to a wash would be
            // three-quarters of a band pretending to work.
            if (tool.draws) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    // Sizes on their own ground at the left, colours opposite: two
                    // questions, and a single row of round things made them look like
                    // one. The same arrangement as the capture editor's.
                    Row(
                        modifier = Modifier
                            .background(
                                MaterialTheme.colorScheme.surfaceVariant,
                                RoundedCornerShape(18.dp),
                            )
                            .padding(horizontal = 4.dp, vertical = 3.dp),
                        horizontalArrangement = Arrangement.spacedBy(2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        ANNOTATION_STROKE_WIDTHS.forEach { width ->
                            val chosen = ANNOTATION_STROKE_WIDTHS.minByOrNull {
                                kotlin.math.abs(it - strokeWidth)
                            } == width
                            Box(
                                modifier = Modifier
                                    .size(34.dp)
                                    .then(
                                        if (chosen) {
                                            Modifier.border(
                                                2.dp,
                                                MaterialTheme.colorScheme.primary,
                                                CircleShape,
                                            )
                                        } else {
                                            Modifier
                                        },
                                    )
                                    .clickable { onStrokeWidth(width) },
                                contentAlignment = Alignment.Center,
                            ) {
                                Canvas(Modifier.size(22.dp)) {
                                    drawCircle(
                                        color = Color(color),
                                        radius = (width * NIB_DOT_SCALE)
                                            .coerceIn(2f, size.minDimension / 2f),
                                    )
                                }
                            }
                        }
                    }

                    LineStylePicker(
                        style = lineStyle,
                        color = color,
                        onStyle = onLineStyle,
                        size = 40.dp,
                    )
                }
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(5.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // The highlighter's washes have to read behind text; ink has to
                // read on white. Same row, different set.
                val palette = if (tool == AnnotationTool.Highlight) {
                    AnnotationColors.highlightPalette
                } else {
                    DRAWING_COLOURS
                }
                // The wheel first, before the six, exactly as in the drawing
                // ribbon: it is the way to *any* colour, so it belongs where the
                // eye starts rather than tucked behind the ones chosen in advance.
                //
                // Offered for the wash too. Six pale colours are what a highlighter
                // usually wants, but "usually" is not "only" — a document already
                // marked up in a house colour needs that colour, and deciding on
                // someone else's behalf that they could not want it was not ours
                // to make.
                CustomColourSwatch(
                    current = color,
                    isCustom = color !in palette,
                    onClick = onPickCustomColour,
                    size = 30.dp,
                )
                palette.forEach { swatch ->
                    ColourDot(
                        colour = swatch,
                        selected = swatch == color,
                        onClick = { onColor(swatch) },
                    )
                }
            }
        }
    }
}

/** The nib sizes on offer, in page points. Fine, medium, heavy. */
internal val ANNOTATION_STROKE_WIDTHS = listOf(1.2f, 2.4f, 5f)

/** How much a nib width is scaled to draw its dot. See the capture editor's. */
private const val NIB_DOT_SCALE = 2.2f

/** How wide the parameters band may grow before its colours start scrolling. */
private val PARAMETER_BAND_WIDTH = 360.dp

/**
 * The words for text already placed on the page.
 *
 * The baseline is chosen first and the words come second, so by the time this
 * opens the place is decided and only the text is missing. It says which kind it
 * is asking for, because a tap and a traced curve are two different gestures with
 * the same dialog at the end of them and nothing else on screen distinguishes the
 * result until the words appear.
 */
@Composable
fun TextComposer(
    curved: Boolean,
    /** The words already there, when an existing caption is being edited. */
    initial: String = "",
    /** Whether this is editing a caption that exists rather than writing a new one. */
    editing: Boolean = false,
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    // Keyed on what came in, so opening the dialog on a different caption shows
    // that caption's words rather than the last one's.
    var text by remember(initial) { mutableStateOf(initial) }
    val focus = remember { FocusRequester() }

    LaunchedEffect(Unit) { focus.requestFocus() }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                when {
                    editing -> "Edit text"
                    curved -> "Text along the curve"
                    else -> "Text"
                },
            )
        },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                placeholder = { Text("Type here") },
                // Several lines, and the return key makes one rather than
                // closing the dialog. A caption is often a sentence that wants
                // breaking, and one long line runs off the sheet.
                singleLine = false,
                minLines = 2,
                maxLines = 6,
                modifier = Modifier.focusRequester(focus),
            )
        },
        confirmButton = {
            // Blank words are how you say "delete this" while editing. Writing a
            // new one, blank adds nothing rather than an invisible mark: text with
            // no letters could only be found by erasing at random.
            TextButton(onClick = { onConfirm(text) }, enabled = editing || text.isNotBlank()) {
                Text(if (editing) "Save" else "Add")
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
