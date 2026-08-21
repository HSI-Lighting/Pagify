package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.MarkupStyle

/**
 * Which line type the next mark is drawn in.
 *
 * One control, used by the reader's tool band and by the capture editor: the same
 * five types mean the same thing on a page and on a picture of a page, and two
 * copies of a picker are two things to keep in step.
 *
 * The glyph *is* the setting — a line in the pattern it selects, rather than a
 * label. "Centerline-2" means nothing until you have seen one, and five names
 * would not fit the slot in any case.
 *
 * A tap opens the list rather than stepping to the next type: with five, reaching
 * the last one by cycling is four taps and a lot of squinting at a small glyph.
 */
@Composable
fun LineStylePicker(
    style: MarkupStyle,
    color: Long,
    onStyle: (MarkupStyle) -> Unit,
    modifier: Modifier = Modifier,
    /** Whether the tool it belongs to is armed, which decides how it is drawn. */
    active: Boolean = true,
    size: Dp = 44.dp,
) {
    var choosing by remember { mutableStateOf(false) }

    Box(modifier) {
        Box(
            modifier = Modifier
                .size(size)
                .background(
                    color = if (active && style != MarkupStyle.SOLID) {
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
                    tint = if (active && style != MarkupStyle.SOLID) {
                        MaterialTheme.colorScheme.onPrimary
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                ),
            contentAlignment = Alignment.Center,
        ) {
            LinePattern(
                style = style,
                tint = if (active && style != MarkupStyle.SOLID) {
                    MaterialTheme.colorScheme.onPrimary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                width = size - 18.dp,
            )
        }

        DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
            MarkupStyle.entries.forEach { option ->
                DropdownMenuItem(
                    text = { Text(option.label) },
                    leadingIcon = {
                        LinePattern(
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
 * Drawn rather than three glyphs, because the dash lengths have to match what
 * will actually be drawn — a picture of a dashed line that dashes differently
 * from the mark is worse than no picture.
 */
@Composable
fun LinePattern(style: MarkupStyle, tint: Color, width: Dp) {
    Canvas(Modifier.size(width, 12.dp)) {
        val thickness = PATTERN_WIDTH_PX
        drawLine(
            color = tint,
            start = Offset(0f, size.height / 2f),
            end = Offset(size.width, size.height / 2f),
            strokeWidth = thickness,
            cap = StrokeCap.Round,
            pathEffect = style.pathEffect(thickness),
        )
    }
}

/** How thick the little pattern is drawn, in pixels. */
private const val PATTERN_WIDTH_PX = 4f
