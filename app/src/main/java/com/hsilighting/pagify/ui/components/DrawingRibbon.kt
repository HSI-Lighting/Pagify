package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowRightAlt
import androidx.compose.material.icons.filled.CheckBoxOutlineBlank
import androidx.compose.material.icons.filled.Gesture
import androidx.compose.material.icons.filled.HorizontalRule
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
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
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.DRAWING_GROUPS
import com.hsilighting.pagify.core.MarkupStyle
import kotlin.math.roundToInt

/**
 * Everything a drawing tool needs, in one row that opens from the ribbon.
 *
 * Six slots: what colour, how heavy, what kind of line, and then the marks
 * themselves — line or arrow, box, and the three ways of going round something.
 * Settings first because they are read left to right as one sentence: *this
 * colour, this weight, this line, drawn as this.*
 *
 * **Every slot opens on a tap.** These used to be a long press each, which meant
 * six things hidden behind a gesture with nothing to say they were there. A slot
 * whose glyph shows a group also shows which member is armed, so the row says
 * what is available as well as what is on.
 *
 * The box is the one slot that arms directly, because it is a group of one and
 * opening a list to choose from a list of one would be a step for nothing.
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
    /** Opens the wheel, for a colour none of the six offers. */
    onPickCustomColour: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf<RibbonPanel?>(null) }

    // Where the slot that opened the panel sits, so the panel opens over it.
    // Window pixels: the panel and the slot are laid out separately, and the
    // window is the space they share.
    var anchorCentre by remember { mutableStateOf(0f) }
    var rowLeft by remember { mutableStateOf(0f) }
    var rowWidth by remember { mutableStateOf(0f) }
    var panelWidth by remember { mutableStateOf(0f) }
    val edgeGap = with(LocalDensity.current) { 8.dp.toPx() }

    val shift = if (rowWidth <= 0f || panelWidth <= 0f) {
        0f
    } else {
        val limit = (rowWidth / 2f - panelWidth / 2f - edgeGap).coerceAtLeast(0f)
        (anchorCentre - (rowLeft + rowWidth / 2f)).coerceIn(-limit, limit)
    }

    fun toggle(panel: RibbonPanel, centre: Float) {
        anchorCentre = centre
        open = if (open == panel) null else panel
    }

    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        open?.let { panel ->
            RibbonPanelSurface(
                modifier = Modifier
                    .offset { IntOffset(shift.roundToInt(), 0) }
                    .onGloballyPositioned { panelWidth = it.size.width.toFloat() },
            ) {
                when (panel) {
                    RibbonPanel.Colour -> ColourChoices(
                        colour = color,
                        onColour = {
                            onColor(it)
                            open = null
                        },
                        onWheel = {
                            open = null
                            onPickCustomColour()
                        },
                    )

                    RibbonPanel.Thickness -> ThicknessChoices(
                        width = strokeWidth,
                        colour = color,
                        onWidth = onStrokeWidth,
                    )

                    RibbonPanel.LineType -> LineTypeChoices(
                        style = lineStyle,
                        onStyle = {
                            onLineStyle(it)
                            open = null
                        },
                    )

                    is RibbonPanel.Tools -> ToolChoices(
                        tools = panel.tools,
                        selectedTool = selectedTool,
                        onTool = {
                            onSelectTool(it)
                            open = null
                        },
                    )
                }
            }
        }

        Surface(
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 3.dp,
            shadowElevation = 8.dp,
            modifier = Modifier.onGloballyPositioned {
                rowLeft = it.boundsInWindow().left
                rowWidth = it.boundsInWindow().width
            },
        ) {
            Row(
                Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RibbonSlot(
                    label = "Colour",
                    onOpen = { toggle(RibbonPanel.Colour, it) },
                ) {
                    ColourGlyph(color)
                }
                RibbonSlot(
                    label = "Thickness",
                    onOpen = { toggle(RibbonPanel.Thickness, it) },
                ) {
                    ThicknessGlyph(strokeWidth)
                }
                RibbonSlot(
                    label = "Line type",
                    onOpen = { toggle(RibbonPanel.LineType, it) },
                ) {
                    LineTypeGlyph(lineStyle)
                }

                DRAWING_GROUPS.forEach { group ->
                    if (group.size == 1) {
                        val only = group.single()
                        // A group of one arms straight away: opening a list to
                        // choose the only thing in it is a step for nothing.
                        RibbonSlot(
                            label = drawingToolName(only),
                            hasMore = false,
                            onOpen = {
                                open = null
                                onSelectTool(
                                    if (selectedTool == only) AnnotationTool.None else only,
                                )
                            },
                        ) {
                            GroupGlyph(group, selectedTool)
                        }
                    } else {
                        RibbonSlot(
                            label = group.joinToString(" or ") { drawingToolName(it) },
                            onOpen = { toggle(RibbonPanel.Tools(group), it) },
                        ) {
                            GroupGlyph(group, selectedTool)
                        }
                    }
                }
            }
        }
    }
}

/** Which panel the row currently has open, if any. */
private sealed interface RibbonPanel {
    data object Colour : RibbonPanel

    data object Thickness : RibbonPanel

    data object LineType : RibbonPanel

    data class Tools(val tools: List<AnnotationTool>) : RibbonPanel
}

/**
 * One slot: a glyph, a tap target, and the wedge that says it opens something.
 *
 * The glyph is passed in rather than named, because half of these are drawn
 * rather than picked from an icon set — a stack of three line weights is not a
 * thing Material has, and a group's glyph has to show which member is armed.
 */
@Composable
private fun RibbonSlot(
    label: String,
    onOpen: (centreX: Float) -> Unit,
    modifier: Modifier = Modifier,
    hasMore: Boolean = true,
    glyph: @Composable () -> Unit,
) {
    var centreX by remember { mutableStateOf(0f) }

    Box(
        modifier = modifier
            .size(SLOT_SIZE)
            .clip(CircleShape)
            .clickable(onClickLabel = label) { onOpen(centreX) }
            .onGloballyPositioned { centreX = it.boundsInWindow().center.x }
            .then(
                if (hasMore) {
                    // The default inset, not a smaller one: the slot clips to a
                    // circle before the wedge is drawn, so a wedge tucked further
                    // into the corner comes out as a sliver of itself.
                    Modifier.longPressHint(tint = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    Modifier
                },
            ),
        contentAlignment = Alignment.Center,
    ) {
        glyph()
    }
}

// ------------------------------------------------------------------- glyphs --

/** The ink, as a disc. The one slot whose glyph *is* its value. */
@Composable
private fun ColourGlyph(colour: Long) {
    Canvas(Modifier.size(26.dp)) {
        drawCircle(Color(colour))
    }
}

/**
 * Three weights stacked, the one in use picked out.
 *
 * Drawn at the weights it actually offers, so the slot is a preview rather than a
 * label: the difference between fine and heavy is the whole point of the control,
 * and three identical lines with a number beside them would say nothing.
 */
@Composable
private fun ThicknessGlyph(width: Float) {
    val accent = RIBBON_ACCENT
    val plain = MaterialTheme.colorScheme.onSurfaceVariant

    Canvas(Modifier.size(30.dp, 22.dp)) {
        val gap = size.height / (ANNOTATION_STROKE_WIDTHS.size + 1)
        ANNOTATION_STROKE_WIDTHS.forEachIndexed { index, weight ->
            val y = gap * (index + 1)
            drawLine(
                color = if (weight == width) accent else plain,
                start = Offset(0f, y),
                end = Offset(size.width, y),
                strokeWidth = weight * GLYPH_WEIGHT_SCALE,
                cap = StrokeCap.Round,
            )
        }
    }
}

/**
 * Three patterns stacked, the family in use picked out.
 *
 * Three rather than five: the two dashes differ only in the length of the dash
 * and the two centre lines only in how many dots, which is a distinction worth
 * making in the panel and not worth making in a 30dp glyph.
 */
@Composable
private fun LineTypeGlyph(style: MarkupStyle) {
    val accent = RIBBON_ACCENT
    val plain = MaterialTheme.colorScheme.onSurfaceVariant
    val rows = listOf(
        MarkupStyle.CENTERLINE_1 to (style == MarkupStyle.CENTERLINE_1 || style == MarkupStyle.CENTERLINE_2),
        MarkupStyle.DASH_1 to (style == MarkupStyle.DASH_1 || style == MarkupStyle.DASH_2),
        MarkupStyle.SOLID to (style == MarkupStyle.SOLID),
    )

    Canvas(Modifier.size(30.dp, 22.dp)) {
        val gap = size.height / (rows.size + 1)
        rows.forEachIndexed { index, (pattern, live) ->
            val y = gap * (index + 1)
            drawLine(
                color = if (live) accent else plain,
                start = Offset(0f, y),
                end = Offset(size.width, y),
                strokeWidth = GLYPH_LINE_PX,
                cap = StrokeCap.Round,
                pathEffect = pattern.pathEffect(GLYPH_LINE_PX),
            )
        }
    }
}

/**
 * Everything in a group at once, with the armed one in the accent.
 *
 * The slot has to answer two questions — what does this offer, and what is on —
 * and showing only the armed one answered just the second. Somebody who has never
 * opened the slot has no way of knowing the cloud is in there.
 */
@Composable
private fun GroupGlyph(group: List<AnnotationTool>, selectedTool: AnnotationTool) {
    when (group.size) {
        1 -> GroupMember(group.single(), selectedTool, 26.dp)

        // Turned upright, and only here. A horizontal bar beside a right-pointing
        // arrow is two wide glyphs in a slot with room for two tall ones, and the
        // pair reads as one arrow with a dash in front of it. Stood on end they
        // read as two things.
        2 -> Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            group.forEach { GroupMember(it, selectedTool, 20.dp, Modifier.rotate(-90f)) }
        }

        // Two above and one below, rather than three in a row: three glyphs
        // across a 46dp slot leaves each of them too small to tell apart.
        else -> Box(Modifier.size(34.dp)) {
            GroupMember(group[0], selectedTool, 15.dp, Modifier.align(Alignment.TopStart))
            GroupMember(group[1], selectedTool, 15.dp, Modifier.align(Alignment.TopEnd))
            GroupMember(group[2], selectedTool, 15.dp, Modifier.align(Alignment.BottomCenter))
        }
    }
}

@Composable
private fun GroupMember(
    tool: AnnotationTool,
    selectedTool: AnnotationTool,
    size: Dp,
    modifier: Modifier = Modifier,
) {
    Icon(
        imageVector = drawingToolGlyph(tool),
        contentDescription = null,
        tint = if (tool == selectedTool) {
            RIBBON_ACCENT
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant
        },
        modifier = modifier.size(size),
    )
}

/**
 * The glyph for one drawing tool.
 *
 * Freehand is the loose squiggle rather than a brush: it is the only one of these
 * whose picture can be the mark itself, and a brush says what you are holding
 * where every other glyph says what you will get.
 */
private fun drawingToolGlyph(tool: AnnotationTool): ImageVector = when (tool) {
    AnnotationTool.Line -> Icons.Filled.HorizontalRule
    AnnotationTool.Arrow -> Icons.AutoMirrored.Filled.ArrowRightAlt
    AnnotationTool.Rectangle -> Icons.Filled.CheckBoxOutlineBlank
    AnnotationTool.Ellipse -> Icons.Filled.RadioButtonUnchecked
    AnnotationTool.Cloud -> Icons.Outlined.Cloud
    else -> Icons.Filled.Gesture
}

private fun drawingToolName(tool: AnnotationTool): String = when (tool) {
    AnnotationTool.Line -> "Line"
    AnnotationTool.Arrow -> "Arrow"
    AnnotationTool.Rectangle -> "Box"
    AnnotationTool.Ellipse -> "Circle"
    AnnotationTool.Cloud -> "Cloud"
    else -> "Freehand"
}

// ------------------------------------------------------------------- panels --

/** The ground every opened panel sits on. */
@Composable
private fun RibbonPanelSurface(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Box(Modifier.padding(horizontal = 10.dp, vertical = 8.dp)) { content() }
    }
}

/** The six, then the way to any other. */
@Composable
private fun ColourChoices(colour: Long, onColour: (Long) -> Unit, onWheel: () -> Unit) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AnnotationColors.markerPalette.forEach { swatch ->
            ColourDot(
                colour = swatch,
                selected = swatch == colour,
                onClick = { onColour(swatch) },
            )
        }
        CustomColourSwatch(
            current = colour,
            isCustom = colour !in AnnotationColors.markerPalette,
            onClick = onWheel,
            size = 30.dp,
        )
    }
}

/**
 * Three weights as taps, and a slider for anything else.
 *
 * The presets are what most marks want and a slider alone would make the common
 * case the slow one; the slider is what makes the uncommon case possible at all.
 */
@Composable
private fun ThicknessChoices(width: Float, colour: Long, onWidth: (Float) -> Unit) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ANNOTATION_STROKE_WIDTHS.forEach { preset ->
                NibDot(
                    width = preset,
                    colour = colour,
                    selected = preset == width,
                    onClick = { onWidth(preset) },
                )
            }
            Text(
                text = "%.1f".format(width),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Slider(
            value = width,
            onValueChange = onWidth,
            valueRange = MINIMUM_STROKE_POINTS..MAXIMUM_STROKE_POINTS,
            modifier = Modifier.width(220.dp),
        )
    }
}

/** One preset weight, drawn as a dot in the ink it will draw with. */
@Composable
private fun NibDot(width: Float, colour: Long, selected: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(34.dp)
            .clip(CircleShape)
            .then(
                if (selected) {
                    Modifier.border(2.dp, RIBBON_ACCENT, CircleShape)
                } else {
                    Modifier
                },
            )
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(22.dp)) {
            drawCircle(
                color = Color(colour),
                radius = (width * NIB_DOT_SCALE).coerceIn(2f, size.minDimension / 2f),
            )
        }
    }
}

/** All five line types, drawn in the pattern they name. */
@Composable
private fun LineTypeChoices(style: MarkupStyle, onStyle: (MarkupStyle) -> Unit) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MarkupStyle.entries.forEach { option ->
            Box(
                modifier = Modifier
                    .size(52.dp, 40.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(
                        if (option == style) {
                            RIBBON_ACCENT
                        } else {
                            Color.Transparent
                        },
                    )
                    .clickable(onClickLabel = option.label) { onStyle(option) },
                contentAlignment = Alignment.Center,
            ) {
                LinePattern(
                    style = option,
                    // Ink on the amber, not the theme's on-primary: that is a
                    // pale colour meant for the theme's own accent, and on this
                    // one it disappears.
                    tint = if (option == style) {
                        ACCENT_INK
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    width = 40.dp,
                )
            }
        }
    }
}

/** The members of a group, to pick one from. */
@Composable
private fun ToolChoices(
    tools: List<AnnotationTool>,
    selectedTool: AnnotationTool,
    onTool: (AnnotationTool) -> Unit,
) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        tools.forEach { tool ->
            val armed = tool == selectedTool
            Box(
                modifier = Modifier
                    .size(SLOT_SIZE)
                    .clip(CircleShape)
                    .background(if (armed) RIBBON_ACCENT else Color.Transparent)
                    // Tapping the armed one puts it down, which is the way out now
                    // that the ribbon slot opens this instead of toggling.
                    .clickable(onClickLabel = drawingToolName(tool)) {
                        onTool(if (armed) AnnotationTool.None else tool)
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = drawingToolGlyph(tool),
                    contentDescription = drawingToolName(tool),
                    tint = if (armed) ACCENT_INK else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(24.dp),
                )
            }
        }
    }
}

/** How wide a slot is. Matches the reader's ribbon, so the two rows line up. */
private val SLOT_SIZE = 46.dp

/**
 * How far a nib width is scaled to draw it in a glyph.
 *
 * The weights are page points, which at glyph size would be a hair's difference
 * between fine and heavy. This is a rank, not a ruler.
 */
private const val GLYPH_WEIGHT_SCALE = 1.6f

/**
 * What marks the live choice, everywhere in this row.
 *
 * A warm amber rather than the theme's own accent. Every slot here is a glyph
 * made of several parts with one of them current, and the thing picked out has to
 * read against a colour swatch, a stack of grey lines and a group of grey icons
 * alike. The theme accent is a blue close enough to the surface tint that "which
 * one is on" had to be worked out rather than seen.
 *
 * Fixed rather than themed for the same reason a highlighter's yellow is fixed:
 * it is not decoration, it is the answer to a question.
 */
private val RIBBON_ACCENT = Color(0xFFF2A93B)

/** What sits *on* the amber. Dark, because the amber is light in either theme. */
private val ACCENT_INK = Color(0xFF241B08)

/** How thick the line-type patterns are drawn, in pixels. */
private const val GLYPH_LINE_PX = 3.5f

/** How far a nib width is scaled to draw its dot. The capture editor's figure. */
private const val NIB_DOT_SCALE = 2.2f

/**
 * What the slider will go down to and up to, in page points.
 *
 * The same range the screenshot editor's slider offers, so a weight means the
 * same thing on a page and on a picture of one.
 */
private const val MINIMUM_STROKE_POINTS = 0.6f
private const val MAXIMUM_STROKE_POINTS = 16f
