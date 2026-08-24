package com.hsilighting.pagify.ui.components

import com.hsilighting.pagify.core.BundledFonts
import androidx.compose.ui.text.font.Typeface
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.PdfFont
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
data class RibbonTool(
    val key: Any,
    val icon: ImageVector,
    val name: String,
    /**
     * Whether the slot's glyph previews this one.
     *
     * A group can hold more than a slot can show. The ones a group is *known* for
     * go in the preview; variations of them are still one tap away in the picker
     * but would only crowd the row. Nothing is hidden by this — the picker always
     * lists the whole group.
     */
    val inPreview: Boolean = true,
)

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
    /**
     * The font, when the armed tool writes words rather than drawing.
     *
     * Non-null swaps two slots: the weight becomes a point size and the line type
     * becomes the font. Neither a nib width nor a dash means anything to a letter,
     * and a row that kept them would be offering controls that do nothing.
     */
    font: PdfFont? = null,
    onFont: (PdfFont) -> Unit = {},
    /**
     * How far the baseline turns from end to end, in degrees, or null when the
     * armed tool writes on a straight line.
     *
     * A slot of its own rather than a mode of the others, because it is a
     * different question — the size says how big, the font says in what, and this
     * says along what.
     */
    curve: Float? = null,
    onCurve: (Float) -> Unit = {},
    onTool: (Any) -> Unit,
    onColour: (Long) -> Unit,
    onWidth: (Float) -> Unit,
    onLineStyle: (MarkupStyle) -> Unit,
    /** Opens the wheel, for a colour the palette does not carry. */
    onPickCustomColour: () -> Unit,
    modifier: Modifier = Modifier,
    widthIsIntensity: Boolean = false,
    onDisarm: (() -> Unit)? = null,
    /**
     * Puts the whole band away. Null where the band is always on screen and
     * there is nothing to close.
     */
    onDismiss: (() -> Unit)? = null,
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
    // out of the row the moment the highlighter is armed, and the font slot only
    // exists while something that writes words is held.
    if (lineStyle == null && open == RibbonPanel.LineType) open = null
    if (font == null && open == RibbonPanel.Font) open = null
    if (curve == null && open == RibbonPanel.Curve) open = null

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
                onClose = { open = null },
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

                    RibbonPanel.Font -> FontChoices(
                        font = font ?: PdfFont.HELVETICA,
                        onFont = {
                            onFont(it)
                            open = null
                        },
                    )

                    RibbonPanel.Curve -> CurveChoices(
                        degrees = curve ?: 0f,
                        onDegrees = onCurve,
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
            // The cross sits outside the scrolling row, not in it: scrolled along
            // with the slots it spent most of its life off the edge of the screen,
            // which is the one thing a way out cannot be.
            Row(verticalAlignment = Alignment.CenterVertically) {
            Row(
                // Scrolled, so the slots keep their own size instead of being
                // squeezed by whatever width is left. A Row given too little
                // measures its last child at nothing: the text slot's glyph went
                // on drawing at full size, over a tap target zero pixels wide, and
                // the tool simply could not be picked. A scrolling row wraps its
                // content while it fits and slides when it does not.
                Modifier
                    .weight(1f, fill = false)
                    .horizontalScroll(rememberScrollState())
                    .padding(start = 8.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RibbonSlot("Colour", { toggle(RibbonPanel.Colour, it) }) { ColourGlyph(colour) }
                // The same two slots ask a different question when what is armed
                // writes words: how big, and in what face.
                RibbonSlot(
                    label = if (font != null) "Size" else "Thickness",
                    onOpen = { toggle(RibbonPanel.Thickness, it) },
                ) {
                    if (font != null) SizeGlyph(width) else ThicknessGlyph(width, widthPresets)
                }
                if (curve != null) {
                    RibbonSlot("Bend", { toggle(RibbonPanel.Curve, it) }) { CurveGlyph(curve) }
                }
                if (font != null) {
                    RibbonSlot("Font", { toggle(RibbonPanel.Font, it) }) { FontGlyph(font) }
                } else if (lineStyle != null) {
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
                            onOpen = { centre ->
                                // A tool in this group is in your hand, so the
                                // slot puts it down. Reaching the armed tool
                                // inside the picker to disarm meant knowing which
                                // of them was armed and finding it again; the slot
                                // is the thing you are already looking at.
                                // Swapping within the group is the second tap.
                                if (group.any { it.key == armed } && onDisarm != null) {
                                    open = null
                                    onDisarm()
                                } else {
                                    toggle(RibbonPanel.Tools(group), centre)
                                }
                            },
                        ) {
                            GroupGlyph(group, armed)
                        }
                    }
                }
            }

            onDismiss?.let { dismiss ->
                RibbonClose {
                    open = null
                    dismiss()
                }
            }
            }
        }
    }
}

/**
 * The band's own way out.
 *
 * Filled, where every slot beside it is not. The cross had to sit in the same row
 * as the tools to be anywhere near the thumb, and in that row a bare glyph reads
 * as one more thing to draw with — so it is given a ground of its own, which is
 * the one visual difference nothing else in the row uses.
 */
@Composable
private fun RibbonClose(onClick: () -> Unit) {
    Box(
        Modifier
            .padding(end = 8.dp)
            .size(CLOSE_SIZE)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(onClickLabel = "Close") { onClick() },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Filled.Close,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(16.dp),
        )
    }
}

/** Which panel the row currently has open, if any. */
private sealed interface RibbonPanel {
    data object Colour : RibbonPanel

    data object Thickness : RibbonPanel

    data object LineType : RibbonPanel

    data object Font : RibbonPanel

    data object Curve : RibbonPanel

    data class Tools(val tools: List<RibbonTool>) : RibbonPanel
}

/** The members a slot previews, at most [PREVIEW_MEMBERS] of them. */
private fun previewOf(group: List<RibbonTool>, armed: Any?): List<RibbonTool> {
    val shown = group.filter { it.inPreview }.take(PREVIEW_MEMBERS)
    if (shown.isEmpty()) return group.take(PREVIEW_MEMBERS)
    if (shown.any { it.key == armed }) return shown
    // Whatever is armed always shows, even when it is one of the ones the row
    // does not normally preview: a slot with nothing lit in it looks like no tool
    // is held at all, and that is the one thing the glyph has to say.
    val held = group.firstOrNull { it.key == armed } ?: return shown
    return shown.dropLast(1) + held
}

private const val PREVIEW_MEMBERS = 4

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
    // A slot is a glyph, not a list. Past four members they stop being tellable
    // apart at 14dp, so the preview shows the first four and makes room for
    // whatever is armed when it is not one of them — otherwise picking the fifth
    // tool would leave the row with nothing lit and no way to see what was held.
    val shown = previewOf(group, armed)
    when (shown.size) {
        1 -> GroupMember(shown.single(), armed, 26.dp)

        // Turned upright, and only here. A horizontal bar beside a right-pointing
        // arrow is two wide glyphs in a slot with room for two tall ones, and the
        // pair reads as one arrow with a dash in front of it. Stood on end they
        // read as two things.
        2 -> Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            shown.forEach { GroupMember(it, armed, 20.dp, Modifier.rotate(-90f)) }
        }

        // Two above and one below, rather than three in a row: three glyphs across
        // a 46dp slot leaves each of them too small to tell apart.
        3 -> Box(Modifier.size(34.dp)) {
            GroupMember(shown[0], armed, 15.dp, Modifier.align(Alignment.TopStart))
            GroupMember(shown[1], armed, 15.dp, Modifier.align(Alignment.TopEnd))
            GroupMember(shown[2], armed, 15.dp, Modifier.align(Alignment.BottomCenter))
        }

        // Four to a corner each. Beyond this a slot stops being a glyph and starts
        // being a list, and the group should be split rather than shrunk further.
        else -> Box(Modifier.size(34.dp)) {
            GroupMember(shown[0], armed, 14.dp, Modifier.align(Alignment.TopStart))
            GroupMember(shown[1], armed, 14.dp, Modifier.align(Alignment.TopEnd))
            GroupMember(shown[2], armed, 14.dp, Modifier.align(Alignment.BottomStart))
            GroupMember(shown[3], armed, 14.dp, Modifier.align(Alignment.BottomEnd))
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
private fun RibbonPanelSurface(
    modifier: Modifier = Modifier,
    /** Shuts the panel. See the cross below for why it is worth the room. */
    onClose: () -> Unit,
    content: @Composable () -> Unit,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Row(
            modifier = Modifier.padding(start = 10.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            content()
            // A way out that is visibly a way out. The panel already closes by
            // tapping the slot that opened it or by choosing from it, but neither
            // of those looks like closing — somebody who opened it to see what was
            // there had to pick something to get rid of it.
            Box(
                Modifier
                    .padding(start = 2.dp)
                    .size(CLOSE_SIZE)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .clickable(onClickLabel = "Close") { onClose() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Filled.Close,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
    }
}

/** Big enough to hit, small enough not to look like one of the choices. */
private val CLOSE_SIZE = 30.dp

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

/**
 * The point size, as the number it is.
 *
 * A number rather than a picture, because that is how a size is asked for: nobody
 * chooses type by pointing at a specimen of it, they say twelve. The other slots
 * are glyphs because their answers have no names anybody uses.
 */
@Composable
private fun SizeGlyph(sizePoints: Float) {
    Text(
        text = sizePoints.roundToInt().toString(),
        style = MaterialTheme.typography.titleMedium,
        color = RIBBON_ACCENT,
    )
}

/** The face, shown in itself — the one label that can be its own specimen. */
@Composable
private fun FontGlyph(font: PdfFont) {
    Text(
        text = "Aa",
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        fontFamily = font.composeFamily(),
        fontWeight = if (font.bold) FontWeight.Bold else FontWeight.Normal,
        style = MaterialTheme.typography.titleMedium,
    )
}

/**
 * The five faces, each written in itself.
 *
 * A list of names in one font tells you nothing about any of them. These are the
 * standard PDF set, so what is on screen is only a likeness — the phone does not
 * have Helvetica — but the difference between a serif and a sans is exactly what
 * the choice is about, and that much a likeness carries.
 */
@Composable
private fun FontChoices(font: PdfFont, onFont: (PdfFont) -> Unit) {
    // Scrolls: there are twenty-odd faces now, and a column of them is taller
    // than the phone.
    Column(
        modifier = Modifier
            .heightIn(max = FONT_LIST_HEIGHT)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        PdfFont.entries.forEach { option ->
            val live = option == font
            Column(
                modifier = Modifier
                    .clip(RoundedCornerShape(10.dp))
                    .background(if (live) RIBBON_ACCENT else Color.Transparent)
                    .clickable { onFont(option) }
                    .padding(horizontal = 14.dp, vertical = 8.dp),
            ) {
                Text(
                    text = option.label,
                    color = if (live) ACCENT_INK else MaterialTheme.colorScheme.onSurface,
                    // The face itself where there is one. Without it a name
                    // written in Devanagari or Korean is drawn by whatever the
                    // phone falls back to, which is the one thing the label was
                    // meant to show.
                    fontFamily = option.pickerFamily(),
                    fontWeight = if (option.bold) FontWeight.Bold else FontWeight.Normal,
                    style = MaterialTheme.typography.bodyLarge,
                )
                Text(
                    text = option.script,
                    color = if (live) {
                        ACCENT_INK.copy(alpha = 0.75f)
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    style = MaterialTheme.typography.labelSmall,
                )
            }
        }
    }
}

/** As tall as the list may get before it starts scrolling instead. */
private val FONT_LIST_HEIGHT = 420.dp

/**
 * The face to draw a font's own name in.
 *
 * The bundled file where there is one, so a name written in its own script is
 * drawn by the font it names. Falling back to the phone's sans-serif would
 * show tofu for half the list.
 */
@Composable
private fun PdfFont.pickerFamily(): FontFamily {
    val typeface = BundledFonts.typefaceFor(this) ?: return composeFamily()
    return remember(this) { FontFamily(Typeface(typeface)) }
}

/** The nearest face the phone has. See [PdfFont.family]. */
private fun PdfFont.composeFamily(): FontFamily = when (family) {
    "serif" -> FontFamily.Serif
    "monospace" -> FontFamily.Monospace
    else -> FontFamily.SansSerif
}

/**
 * How far the line bends, as a row of shapes and a slider.
 *
 * The presets are the three answers most captions want — sagging, straight,
 * arching — and the slider is there for the one that wants something else.
 */
@Composable
private fun CurveChoices(degrees: Float, onDegrees: (Float) -> Unit) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CURVE_PRESETS.forEach { preset ->
                Box(
                    Modifier
                        .size(38.dp)
                        .clip(CircleShape)
                        .clickable { onDegrees(preset) },
                    contentAlignment = Alignment.Center,
                ) {
                    CurveGlyph(preset, lit = preset == degrees, size = 26.dp)
                }
            }
            Text(
                text = "${degrees.roundToInt()}°",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Slider(
            value = degrees,
            onValueChange = onDegrees,
            valueRange = -CURVE_LIMIT..CURVE_LIMIT,
            modifier = Modifier.width(220.dp),
        )
    }
}

/** The bend itself, drawn: an arc turning through the amount it stands for. */
@Composable
private fun CurveGlyph(degrees: Float, lit: Boolean = true, size: Dp = 24.dp) {
    val ink = if (lit) RIBBON_ACCENT else MaterialTheme.colorScheme.onSurfaceVariant
    Canvas(Modifier.size(size)) {
        val turn = Math.toRadians(degrees.toDouble()).toFloat()
        val span = this.size.width * 0.82f
        val left = (this.size.width - span) / 2f
        val middle = this.size.height / 2f
        val path = Path()

        if (abs(turn) < 0.02f) {
            path.moveTo(left, middle)
            path.lineTo(left + span, middle)
        } else {
            // The same arithmetic the baseline uses, so the glyph is a preview of
            // the line rather than a picture of one.
            val radius = span / turn
            val start = -turn / 2f
            val steps = 24
            for (step in 0..steps) {
                val along = start + turn * step / steps
                val x = left + radius * (sin(along) - sin(start))
                val y = middle + radius * (cos(start) - cos(along))
                if (step == 0) path.moveTo(x, y) else path.lineTo(x, y)
            }
        }

        drawPath(
            path = path,
            color = ink,
            style = Stroke(width = this.size.width * 0.09f, cap = StrokeCap.Round),
        )
    }
}

/** Sag, straight, arch: the three bends a caption usually wants. */
private val CURVE_PRESETS = listOf(-60f, 0f, 60f)

/** Past half a turn the words start meeting themselves. */
private const val CURVE_LIMIT = 180f
