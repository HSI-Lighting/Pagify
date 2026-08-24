package com.hsilighting.pagify.ui.components

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Gesture
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material.icons.filled.HorizontalRule
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.DRAWING_GROUPS
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MINIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.MAXIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.bendsText
import com.hsilighting.pagify.core.writesText

/**
 * The reader's drawing tools, in the shared ribbon.
 *
 * Everything about how the row behaves lives in [MarkRibbon]; this only says which
 * tools the reader has and what they are called. The screenshot editor has its own
 * few lines saying the same for its own.
 */
@Composable
fun DrawingRibbon(
    selectedTool: AnnotationTool,
    color: Long,
    strokeWidth: Float,
    lineStyle: MarkupStyle,
    font: PdfFont,
    sizePoints: Float,
    curveDegrees: Float,
    /** The largest size that still fits across the page. */
    sizeCeiling: Float,
    /** Whether the bend still means anything for the caption in hand. */
    bendApplies: Boolean,
    onFont: (PdfFont) -> Unit,
    onCurve: (Float) -> Unit,
    onSizePoints: (Float) -> Unit,
    onSelectTool: (AnnotationTool) -> Unit,
    onColor: (Long) -> Unit,
    onStrokeWidth: (Float) -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    onPickCustomColour: () -> Unit,
    /** Puts the band away. Null where it is always on screen. */
    onDismiss: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
) {
    MarkRibbon(
        onDismiss = onDismiss,
        groups = DRAWING_GROUPS.map { group ->
            group.map {
                RibbonTool(
                    key = it,
                    icon = drawingToolGlyph(it),
                    name = drawingToolName(it),
                    // The box and the ellipse are the cloud with a different ring
                    // round the words. Showing all three in one slot says nothing
                    // the cloud does not already say, and costs the other members
                    // the room to be legible.
                    inPreview = it != AnnotationTool.BoxText && it != AnnotationTool.EllipseText,
                )
            }
        },
        armed = selectedTool,
        colour = color,
        palette = AnnotationColors.markerPalette,
        // The weight slot is a point size while text is armed, so it has to be
        // fed the size and hand back the size — the row asks one question with
        // one control, and which question it is depends on what is held.
        width = if (selectedTool.writesText) sizePoints else strokeWidth,
        widthPresets = if (selectedTool.writesText) TEXT_SIZES else ANNOTATION_STROKE_WIDTHS,
        widthRange = if (selectedTool.writesText) {
            MINIMUM_TEXT_POINTS..sizeCeiling
        } else {
            MINIMUM_STROKE_POINTS..MAXIMUM_STROKE_POINTS
        },
        lineStyle = lineStyle.takeIf { !selectedTool.writesText },
        font = font.takeIf { selectedTool.writesText },
        onFont = onFont,
        // Only while a tool that bends is held: a straight caption has no bend to
        // set, and a slot that does nothing is worse than no slot.
        // Gone once the caption has more than one line: a block does not bend,
        // so the slot would be a control that does nothing.
        curve = curveDegrees.takeIf { selectedTool.bendsText && bendApplies },
        onCurve = onCurve,
        onTool = { onSelectTool(it as AnnotationTool) },
        onColour = onColor,
        onWidth = if (selectedTool.writesText) onSizePoints else onStrokeWidth,
        onLineStyle = onLineStyle,
        onPickCustomColour = onPickCustomColour,
        modifier = modifier,
        // The reader can put every tool down and go back to plain scrolling. A
        // tool that can only be turned on is a trap, since every touch would keep
        // drawing.
        onDisarm = { onSelectTool(AnnotationTool.None) },
    )
}

/**
 * The glyph for one drawing tool.
 *
 * Freehand is the loose squiggle rather than a brush: it is the only one of these
 * whose picture can be the mark itself, and a brush says what you are holding
 * where every other glyph says what you will get.
 */
internal fun drawingToolGlyph(tool: AnnotationTool): ImageVector = when (tool) {
    AnnotationTool.Line -> Icons.Filled.HorizontalRule
    AnnotationTool.Arrow -> Icons.AutoMirrored.Filled.ArrowRightAlt
    AnnotationTool.Rectangle -> Icons.Filled.CheckBoxOutlineBlank
    AnnotationTool.Ellipse -> Icons.Filled.RadioButtonUnchecked
    AnnotationTool.Cloud -> Icons.Outlined.Cloud
    AnnotationTool.Text -> Icons.Filled.TextFields
    AnnotationTool.CurvedText -> CurvedTextIcon
    AnnotationTool.CloudText -> CloudTextIcon
    AnnotationTool.BoxText -> BoxTextIcon
    AnnotationTool.EllipseText -> EllipseTextIcon
    AnnotationTool.Curve -> CurvedLineIcon
    AnnotationTool.CurvedArrow -> CurvedArrowIcon
    else -> Icons.Filled.Gesture
}

internal fun drawingToolName(tool: AnnotationTool): String = when (tool) {
    AnnotationTool.Line -> "Line"
    AnnotationTool.Arrow -> "Arrow"
    AnnotationTool.Rectangle -> "Box"
    AnnotationTool.Ellipse -> "Circle"
    AnnotationTool.Cloud -> "Cloud"
    AnnotationTool.Text -> "Text"
    AnnotationTool.CurvedText -> "Curved text"
    AnnotationTool.CloudText -> "Clouded text"
    AnnotationTool.BoxText -> "Boxed text"
    AnnotationTool.EllipseText -> "Circled text"
    AnnotationTool.Curve -> "Curved line"
    AnnotationTool.CurvedArrow -> "Curved arrow"
    else -> "Freehand"
}

/**
 * What the reader's slider will go down to and up to, in page points.
 *
 * The same range the screenshot editor's offers, so a weight means the same thing
 * on a page and on a picture of one.
 */
private const val MINIMUM_STROKE_POINTS = 0.6f
private const val MAXIMUM_STROKE_POINTS = 16f

/**
 * The sizes offered as a tap, in points.
 *
 * Printers' sizes rather than round numbers: these are the ones type actually
 * comes in, and somebody marking up a drawing is usually matching something
 * already on it.
 */
internal val TEXT_SIZES = listOf(9f, 12f, 18f)

/** What the size slider will go down to and up to, in points. */


