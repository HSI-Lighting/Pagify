package com.hsilighting.pagify.ui.components

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Gesture
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
    onSelectTool: (AnnotationTool) -> Unit,
    onColor: (Long) -> Unit,
    onStrokeWidth: (Float) -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    onPickCustomColour: () -> Unit,
    modifier: Modifier = Modifier,
) {
    MarkRibbon(
        groups = DRAWING_GROUPS.map { group ->
            group.map { RibbonTool(it, drawingToolGlyph(it), drawingToolName(it)) }
        },
        armed = selectedTool,
        colour = color,
        palette = AnnotationColors.markerPalette,
        width = strokeWidth,
        widthPresets = ANNOTATION_STROKE_WIDTHS,
        widthRange = MINIMUM_STROKE_POINTS..MAXIMUM_STROKE_POINTS,
        lineStyle = lineStyle,
        onTool = { onSelectTool(it as AnnotationTool) },
        onColour = onColor,
        onWidth = onStrokeWidth,
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
