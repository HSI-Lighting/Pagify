package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Gesture
import androidx.compose.material.icons.filled.Highlight
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material.icons.filled.HorizontalRule
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MINIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.MAXIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.bendsText
import com.hsilighting.pagify.core.writesText
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.hasLineStyle
import com.hsilighting.pagify.core.isIntensity
import com.hsilighting.pagify.core.sizePresets
import com.hsilighting.pagify.core.sizeRange

/** What the export sheet is being opened for. Only the verb differs. */
internal enum class ExportAction(val verb: String) {
    Save("Save"),
    Share("Share"),
    Copy("Copy"),
}

/**
 * The screenshot editor's tools, in the shared ribbon.
 *
 * The same row the reader draws with, because these are the same questions — what
 * colour, how heavy, what kind of line, what shape. Two rows that looked alike but
 * were built separately is exactly how the two would have drifted apart.
 *
 * The highlighter is the one tool the reader's row does not have, and it takes a
 * slot of its own: it is neither a line nor a way of going round something.
 */
@Composable
internal fun MarkupRibbon(
    tool: MarkupTool,
    /** Whether that tool is held. Nothing is picked out in the row when it is not. */
    armed: Boolean,
    onDisarm: () -> Unit,
    color: Long,
    size: Float,
    style: MarkupStyle,
    font: PdfFont,
    sizePoints: Float,
    onTool: (MarkupTool) -> Unit,
    onColor: (Long) -> Unit,
    onSize: (MarkupTool, Float) -> Unit,
    onStyle: (MarkupStyle) -> Unit,
    curveDegrees: Float,
    onFont: (PdfFont) -> Unit,
    onCurve: (Float) -> Unit,
    onSizePoints: (Float) -> Unit,
    onPickCustomColour: () -> Unit,
    modifier: Modifier = Modifier,
) {
    MarkRibbon(
        groups = MARKUP_GROUPS.map { group ->
            group.map {
                RibbonTool(
                    key = it,
                    icon = markupToolIcon(it),
                    name = markupToolLabel(it),
                    // As in the reader: the box and the ellipse are the cloud
                    // with a different ring, and showing all five in one slot
                    // costs the others the room to be legible.
                    inPreview = it != MarkupTool.BoxText && it != MarkupTool.EllipseText,
                )
            }
        },
        armed = tool.takeIf { armed },
        colour = color,
        palette = MARKUP_COLOURS,
        // While words are held the weight slot is a point size and the line-type
        // slot is a font, exactly as in the reader — neither a nib width nor a
        // dash means anything to a letter.
        font = font.takeIf { tool.writesText },
        width = if (tool.writesText) sizePoints else size,
        widthPresets = if (tool.writesText) TEXT_SIZES else tool.sizePresets,
        widthRange = if (tool.writesText) {
            MINIMUM_TEXT_POINTS..MAXIMUM_TEXT_POINTS
        } else {
            tool.sizeRange
        },
        // A wash has no length to break up, so the slot drops out rather than
        // offering five patterns that would every one of them draw the same block.
        lineStyle = style.takeIf { tool.hasLineStyle && !tool.writesText },
        onTool = { onTool(it as MarkupTool) },
        onColour = onColor,
        onWidth = { if (tool.writesText) onSizePoints(it) else onSize(tool, it) },
        onFont = onFont,
        curve = curveDegrees.takeIf { tool.bendsText },
        onCurve = onCurve,
        onLineStyle = onStyle,
        onPickCustomColour = onPickCustomColour,
        modifier = modifier,
        // Same control, different question: the highlighter's is how strong the
        // wash is, not how thick the nib.
        widthIsIntensity = tool.isIntensity && !tool.writesText,
        // A tool can be put down here, as in the reader. It was not, on the
        // reasoning that a finger on the picture had nothing else it could mean —
        // but while pinching to zoom, a finger that lands a moment before or after
        // its partner is one finger, and every one of those drew on the picture.
        onDisarm = onDisarm,
    )
}

/**
 * The markup tools, grouped as the ribbon offers them.
 *
 * The reader's grouping, plus the highlighter. Grouped by what the mark *is*: a
 * line and an arrow are one question, and so are the three ways of going round
 * something.
 */
private val MARKUP_GROUPS: List<List<MarkupTool>> = listOf(
    listOf(
        MarkupTool.Line,
        MarkupTool.Arrow,
        MarkupTool.Curve,
        MarkupTool.CurvedArrow,
    ),
    listOf(MarkupTool.Rectangle),
    listOf(MarkupTool.Pen, MarkupTool.Ellipse, MarkupTool.Cloud),
    listOf(MarkupTool.Highlight),
    listOf(
        MarkupTool.Text,
        MarkupTool.CurvedText,
        MarkupTool.CloudText,
        MarkupTool.BoxText,
        MarkupTool.EllipseText,
    ),
)

/**
 * How sharp, what kind of file, and what fills the bare parts — asked once, at the
 * moment it matters.
 *
 * These used to sit on screen the whole time a picture was being marked up, which
 * put three questions about the *file* in front of somebody drawing an arrow.
 * Worse, the sharpness was offered as 1×, 2× and 4× — multiples of the page's
 * natural size, which is not a thing anybody sees, so there was no way to tell
 * which one you wanted.
 *
 * The answers are remembered, so somebody who always sends the same kind of file
 * is confirming a sheet that is already right rather than answering again.
 */
@Composable
internal fun ExportSheet(
    action: ExportAction,
    scale: CaptureScale,
    format: CaptureFormat,
    fill: CaptureFill,
    /** Whether the picture has any area no page reaches. */
    hasBareArea: Boolean,
    isCapturing: Boolean,
    onScale: (CaptureScale) -> Unit,
    onFormat: (CaptureFormat) -> Unit,
    onFill: (CaptureFill) -> Unit,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("${action.verb} this picture") },
        confirmButton = {
            // Held back while the picture is being re-rendered: changing the
            // sharpness takes a fresh capture, and exporting before it lands would
            // hand over a file that is not the one that was asked for.
            Button(onClick = onConfirm, enabled = !isCapturing) { Text(action.verb) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                ExportChoice("Quality") {
                    CaptureScale.entries.forEach { option ->
                        FilterChip(
                            selected = scale == option,
                            onClick = { onScale(option) },
                            enabled = !isCapturing,
                            label = { Text(option.label) },
                        )
                    }
                }

                ExportChoice("Format") {
                    CaptureFormat.entries.forEach { option ->
                        FilterChip(
                            selected = format == option,
                            onClick = { onFormat(option) },
                            enabled = !isCapturing,
                            label = { Text(option.extension.uppercase()) },
                        )
                    }
                }

                // Only for a picture that has an outside. A box capture inside one
                // page is all page, so the question would have nothing to act on.
                if (hasBareArea) {
                    ExportChoice("Around it") {
                        CaptureFill.entries.forEach { option ->
                            FilterChip(
                                selected = fill == option,
                                onClick = { onFill(option) },
                                enabled = !isCapturing,
                                label = { Text(option.label) },
                            )
                        }
                    }
                }
            }
        },
    )
}

/** One labelled row of chips in the export sheet. */
@Composable
private fun ExportChoice(label: String, choices: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(
            modifier = Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            choices()
        }
    }
}

/** The glyph for a markup tool. Shapes, because a shape reads faster than a word. */
private fun markupToolIcon(tool: MarkupTool): ImageVector = when (tool) {
    MarkupTool.Line -> Icons.Filled.HorizontalRule
    MarkupTool.Arrow -> Icons.AutoMirrored.Filled.ArrowRightAlt
    MarkupTool.Rectangle -> Icons.Filled.CheckBoxOutlineBlank
    MarkupTool.Ellipse -> Icons.Filled.RadioButtonUnchecked
    MarkupTool.Cloud -> Icons.Outlined.Cloud
    MarkupTool.Curve -> CurvedLineIcon
    MarkupTool.CurvedArrow -> CurvedArrowIcon
    MarkupTool.Highlight -> Icons.Filled.Highlight
    MarkupTool.Pen -> Icons.Filled.Gesture
    MarkupTool.Text -> Icons.Filled.TextFields
    MarkupTool.CurvedText -> CurvedTextIcon
    MarkupTool.CloudText -> CloudTextIcon
    MarkupTool.BoxText -> BoxTextIcon
    MarkupTool.EllipseText -> EllipseTextIcon
}

private fun markupToolLabel(tool: MarkupTool): String = when (tool) {
    MarkupTool.Line -> "Line"
    MarkupTool.Arrow -> "Arrow"
    MarkupTool.Rectangle -> "Box"
    MarkupTool.Ellipse -> "Circle"
    MarkupTool.Cloud -> "Cloud"
    MarkupTool.Curve -> "Curved line"
    MarkupTool.CurvedArrow -> "Curved arrow"
    MarkupTool.Highlight -> "Highlight"
    MarkupTool.Text -> "Text"
    MarkupTool.CurvedText -> "Curved text"
    MarkupTool.CloudText -> "Clouded text"
    MarkupTool.BoxText -> "Boxed text"
    MarkupTool.EllipseText -> "Circled text"
    MarkupTool.Pen -> "Freehand"
}
