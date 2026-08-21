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
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.MarkupStyle
import kotlin.math.roundToInt

/**
 * One tool, as the ribbon needs to know it.
 *
 * Untyped on purpose. The reader marks a page and the screenshot editor marks a
 * picture, and they have separate tool enums because they can do separate things
 * — but the *row* is the same row, and a second copy of it would be the place the
 * two quietly drifted apart. [key] is whichever enum value the caller uses; the
 * ribbon only ever compares it and hands it back.
 */
data class RibbonTool(val key: Any, val icon: ImageVector, val name: String)

/**
 * Everything a mark-making tool needs, in one row.
 *
 * Colour, weight, line type, then the marks themselves. Read left to right it is
 * one sentence: *this colour, this weight, this line, drawn as this.*
 *
 * **Every slot opens on a tap.** These were a long press each, which meant six
 * things hidden behind a gesture with nothing to say they were there. A slot whose
 * glyph shows a group also shows which member is armed, so the row says what is
 * available as well as what is on — somebody who has never opened it can still see
 * what is in there.
 *
 * A group of one arms directly instead of opening: a list to choose from a list of
 * one is a step for nothing.
 *
 * @param lineStyle null when the armed tool has no line type, and then the slot
 *   drops out of the row entirely rather than opening a panel that changes
 *   nothing. The highlighter is the case: a wash has no length to break up.
 * @param widthIsIntensity the weight slot means "how strong", not "how thick".
 *   Same control, same question — only the range and the way it reads differ.
 * @param onDisarm null where a tool is always held. The reader can put every tool
 *   down and go back to scrolling; the screenshot editor cannot, because a finger
 *   on the picture there has nothing else to mean.
 */
@Composable
fun MarkRibbon(
    groups: List<List<RibbonTool>>,
    armed: Any?,
    colour: Long,
    palette: List<Long>,
    width: Float,
    widthPresets: List<Float>,
    widthRange: ClosedFloatingPointRange<Float>,
    lineStyle: MarkupStyle?,
    onTool: (Any) -> Unit,
    onColour: (Long) -> Unit,
    onWidth: (Float) -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    /** Opens the wheel, for a colour the palette does not carry. */
    onPickCustomColour: () -> Unit,
    modifier: Modifier = Modifier,
    widthIsIntensity: Boolean = false,
    onDisarm: (() -> Unit)? = null,
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

    // A slot that vanishes must not leave its panel behind: the line type drops
    // out of the row the moment the highlighter is armed.
    if (lineStyle == null && open == RibbonPanel.LineType) open = null

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
                        colour = colour,
                        palette = palette,
                        onColour = {
                            onColour(it)
                            open = null
                        },
                        onWheel = {
                            open = null
                            onPickCustomColour()
                        },
                    )

                    RibbonPanel.Thickness -> ThicknessChoices(
                        width = width,
                        colour = colour,
                        presets = widthPresets,
                        range = widthRange,
                        isIntensity = widthIsIntensity,
                        onWidth = onWidth,
                    )

                    RibbonPanel.LineType -> LineTypeChoices(
                        style = lineStyle ?: MarkupStyle.SOLID,
                        onStyle = {
                            onLineStyle(it)
                            open = null
                        },
                    )

                    is RibbonPanel.Tools -> ToolChoices(
                        tools = panel.tools,
                        armed = armed,
                        onTool = {
                            onTool(it)
                            open = null
                        },
                        onDisarm = onDisarm?.let {
                            {
                                it()
                                open = null
                            }
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
                RibbonSlot("Colour", { toggle(RibbonPanel.Colour, it) }) { ColourGlyph(colour) }
                RibbonSlot("Thickness", { toggle(RibbonPanel.Thickness, it) }) {
                    ThicknessGlyph(width, widthPresets)
                }
                if (lineStyle != null) {
                    RibbonSlot("Line type", { toggle(RibbonPanel.LineType, it) }) {
                        LineTypeGlyph(lineStyle)
                    }
                }

                groups.forEach { group ->
                    if (group.size == 1) {
                        val only = group.single()
                        RibbonSlot(
                            label = only.name,
                            hasMore = false,
                            onOpen = {
                                open = null
                                if (only.key == armed && onDisarm != null) {
                                    onDisarm()
                                } else {
                                    onTool(only.key)
                                }
                            },
                        ) {
                            GroupGlyph(group, armed)
                        }
                    } else {
                        RibbonSlot(
                            label = group.joinToString(" or ") { it.name },
                            onOpen = { toggle(RibbonPanel.Tools(group), it) },
                        ) {
                            GroupGlyph(group, armed)
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

    data class Tools(val tools: List<RibbonTool>) : RibbonPanel
}

/**
 * One slot: a glyph, a tap target, and the wedge that says it opens something.
 *
 * The glyph is passed in rather than named, because half of these are drawn rather
 * than picked from an icon set — a stack of three line weights is not a thing
 * Material has, and a group's glyph has to show which member is armed.
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
    Canvas(Modifier.size(26.dp)) { drawCircle(Color(colour)) }
}

/**
 * The weights on offer, stacked, with the one in use picked out.
 *
 * Drawn at the weights it actually offers, so the slot is a preview rather than a
 * label: the difference between fine and heavy is the whole point of the control,
 * and three identical lines with a number beside them would say nothing.
 */
@Composable
private fun ThicknessGlyph(width: Float, presets: List<Float>) {
    val plain = MaterialTheme.colorScheme.onSurfaceVariant
    val heaviest = presets.maxOrNull() ?: 1f

    Canvas(Modifier.size(30.dp, 22.dp)) {
        val gap = size.height / (presets.size + 1)
        presets.forEachIndexed { index, weight ->
            val y = gap * (index + 1)
            drawLine(
                color = if (weight == width) RIBBON_ACCENT else plain,
                start = Offset(0f, y),
                end = Offset(size.width, y),
                // Scaled against the heaviest on offer rather than by a constant,
                // because the two ribbons measure different things: page points in
                // one, a fraction of full strength in the other.
                strokeWidth = (weight / heaviest * GLYPH_HEAVIEST_PX).coerceAtLeast(1.5f),
                cap = StrokeCap.Round,
            )
        }
    }
}

/**
 * Three patterns stacked, the family in use picked out.
 *
 * Three rather than five: the two dashes differ only in the length of the dash and
 * the two centre lines only in how many dots, which is a distinction worth making
 * in the panel and not worth making in a 30dp glyph.
 */
@Composable
private fun LineTypeGlyph(style: MarkupStyle) {
    val plain = MaterialTheme.colorScheme.onSurfaceVariant
    val rows = listOf(
        MarkupStyle.CENTERLINE_1 to
            (style == MarkupStyle.CENTERLINE_1 || style == MarkupStyle.CENTERLINE_2),
        MarkupStyle.DASH_1 to (style == MarkupStyle.DASH_1 || style == MarkupStyle.DASH_2),
        MarkupStyle.SOLID to (style == MarkupStyle.SOLID),
    )

    Canvas(Modifier.size(30.dp, 22.dp)) {
        val gap = size.height / (rows.size + 1)
        rows.forEachIndexed { index, (pattern, live) ->
            val y = gap * (index + 1)
            drawLine(
                color = if (live) RIBBON_ACCENT else plain,
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
 * and showing only the armed one answered just the second.
 */
@Composable
private fun GroupGlyph(group: List<RibbonTool>, armed: Any?) {
    when (group.size) {
        1 -> GroupMember(group.single(), armed, 26.dp)

        // Turned upright, and only here. A horizontal bar beside a right-pointing
        // arrow is two wide glyphs in a slot with room for two tall ones, and the
        // pair reads as one arrow with a dash in front of it. Stood on end they
        // read as two things.
        2 -> Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            group.forEach { GroupMember(it, armed, 20.dp, Modifier.rotate(-90f)) }
        }

        // Two above and one below, rather than three in a row: three glyphs across
        // a 46dp slot leaves each of them too small to tell apart.
        3 -> Box(Modifier.size(34.dp)) {
            GroupMember(group[0], armed, 15.dp, Modifier.align(Alignment.TopStart))
            GroupMember(group[1], armed, 15.dp, Modifier.align(Alignment.TopEnd))
            GroupMember(group[2], armed, 15.dp, Modifier.align(Alignment.BottomCenter))
        }

        // Four to a corner each. Beyond this a slot stops being a glyph and starts
        // being a list, and the group should be split rather than shrunk further.
        else -> Box(Modifier.size(34.dp)) {
            GroupMember(group[0], armed, 14.dp, Modifier.align(Alignment.TopStart))
            GroupMember(group[1], armed, 14.dp, Modifier.align(Alignment.TopEnd))
            GroupMember(group[2], armed, 14.dp, Modifier.align(Alignment.BottomStart))
            GroupMember(group[3], armed, 14.dp, Modifier.align(Alignment.BottomEnd))
        }
    }
}

@Composable
private fun GroupMember(
    tool: RibbonTool,
    armed: Any?,
    size: Dp,
    modifier: Modifier = Modifier,
) {
    Icon(
        imageVector = tool.icon,
        contentDescription = null,
        tint = if (tool.key == armed) {
            RIBBON_ACCENT
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant
        },
        modifier = modifier.size(size),
    )
}

// ------------------------------------------------------------------- panels --

/** The ground every opened panel sits on. */
@Composable
private fun RibbonPanelSurface(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
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

/** The palette, then the way to any other colour. */
@Composable
private fun ColourChoices(
    colour: Long,
    palette: List<Long>,
    onColour: (Long) -> Unit,
    onWheel: () -> Unit,
) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // The wheel first: it is the way to *any* colour, so it belongs where the
        // eye starts rather than tucked behind the ones chosen in advance.
        CustomColourSwatch(
            current = colour,
            isCustom = colour !in palette,
            onClick = onWheel,
            size = 30.dp,
        )
        palette.forEach { swatch ->
            ColourDot(colour = swatch, selected = swatch == colour, onClick = { onColour(swatch) })
        }
    }
}

/**
 * The presets as taps, and a slider for anything else.
 *
 * The presets are what most marks want and a slider alone would make the common
 * case the slow one; the slider is what makes the uncommon case possible at all.
 */
@Composable
private fun ThicknessChoices(
    width: Float,
    colour: Long,
    presets: List<Float>,
    range: ClosedFloatingPointRange<Float>,
    isIntensity: Boolean,
    onWidth: (Float) -> Unit,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            presets.forEach { preset ->
                NibDot(
                    width = preset,
                    heaviest = presets.maxOrNull() ?: 1f,
                    colour = colour,
                    selected = preset == width,
                    onClick = { onWidth(preset) },
                )
            }
            Text(
                text = if (isIntensity) {
                    "${(width * 100).roundToInt()}%"
                } else {
                    "%.1f".format(width)
                },
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Slider(
            value = width,
            onValueChange = onWidth,
            valueRange = range,
            modifier = Modifier.width(220.dp),
        )
    }
}

/** One preset, drawn as a dot in the ink it will draw with. */
@Composable
private fun NibDot(
    width: Float,
    heaviest: Float,
    colour: Long,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Box(
        modifier = Modifier
            .size(34.dp)
            .clip(CircleShape)
            .then(
                if (selected) Modifier.border(2.dp, RIBBON_ACCENT, CircleShape) else Modifier,
            )
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(22.dp)) {
            drawCircle(
                color = Color(colour),
                // A rank, not a ruler: the presets are page points in one ribbon
                // and a fraction in the other, so the dot is drawn relative to the
                // heaviest on offer rather than at a fixed scale.
                radius = (width / heaviest * size.minDimension / 2f).coerceAtLeast(2f),
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
                    .background(if (option == style) RIBBON_ACCENT else Color.Transparent)
                    .clickable(onClickLabel = option.label) { onStyle(option) },
                contentAlignment = Alignment.Center,
            ) {
                LinePattern(
                    style = option,
                    // Ink on the amber, not the theme's on-primary: that is a pale
                    // colour meant for the theme's own accent, and on this one it
                    // disappears.
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
    tools: List<RibbonTool>,
    armed: Any?,
    onTool: (Any) -> Unit,
    onDisarm: (() -> Unit)?,
) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        tools.forEach { tool ->
            val live = tool.key == armed
            Box(
                modifier = Modifier
                    .size(SLOT_SIZE)
                    .clip(CircleShape)
                    .background(if (live) RIBBON_ACCENT else Color.Transparent)
                    .clickable(onClickLabel = tool.name) {
                        // Tapping the armed one puts it down, where putting it down
                        // is a thing that can be done at all.
                        if (live && onDisarm != null) onDisarm() else onTool(tool.key)
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = tool.icon,
                    contentDescription = tool.name,
                    tint = if (live) ACCENT_INK else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(24.dp),
                )
            }
        }
    }
}

/** How wide a slot is. Matches the reader's ribbon, so the two rows line up. */
private val SLOT_SIZE = 46.dp

/**
 * What marks the live choice, everywhere in this row.
 *
 * A warm amber rather than the theme's own accent. Every slot here is a glyph made
 * of several parts with one of them current, and the thing picked out has to read
 * against a colour swatch, a stack of grey lines and a group of grey icons alike.
 * The theme accent is a blue close enough to the surface tint that "which one is
 * on" had to be worked out rather than seen.
 *
 * Fixed rather than themed for the same reason a highlighter's yellow is fixed: it
 * is not decoration, it is the answer to a question.
 */
private val RIBBON_ACCENT = Color(0xFFF2A93B)

/** What sits *on* the amber. Dark, because the amber is light in either theme. */
private val ACCENT_INK = Color(0xFF241B08)

/** How thick the heaviest weight is drawn in a glyph, in pixels. */
private const val GLYPH_HEAVIEST_PX = 7f

/** How thick the line-type patterns are drawn, in pixels. */
private const val GLYPH_LINE_PX = 3.5f
