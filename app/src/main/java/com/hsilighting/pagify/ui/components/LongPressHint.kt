package com.hsilighting.pagify.ui.components

import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * The mark that says "there is more under this one".
 *
 * A small wedge in the bottom-right corner, the way a menu that opens on a long
 * press has been signposted since long before Android. It exists because a long
 * press is invisible: a control that only answers to one is a control most people
 * never find, and every hidden thing in this app — the pen's palette, the eraser's
 * clear menu, a tool's size slider — was reachable and undiscoverable at once.
 *
 * Drawn rather than an icon: it is three points, it has to sit exactly in the
 * corner of whatever it marks, and a glyph would bring its own padding and its own
 * baseline to argue with.
 *
 * Applied to the *icon's* box, so it stays put whatever the icon inside is doing —
 * a tool that swaps its glyph with the mode it is in keeps the same wedge in the
 * same place.
 */
fun Modifier.longPressHint(
    tint: Color,
    size: Dp = HINT_SIZE,
    inset: Dp = HINT_INSET,
): Modifier = drawWithContent {
    drawContent()

    val side = size.toPx()
    val edge = inset.toPx()
    val right = this.size.width - edge
    val bottom = this.size.height - edge

    // A right triangle filling the corner: across, down, and back to the point.
    // The hypotenuse faces up and left, which is what makes it read as an arrow
    // aimed at the corner rather than as a stray dot.
    val wedge = Path().apply {
        moveTo(right, bottom - side)
        lineTo(right, bottom)
        lineTo(right - side, bottom)
        close()
    }

    drawPath(wedge, tint)
}

/**
 * How big the wedge is.
 *
 * Small enough to be a hint rather than a second icon, large enough to survive
 * being drawn on a dense screen — below about 5dp it reads as a rendering
 * artefact and people stop seeing it as deliberate.
 */
private val HINT_SIZE = 6.dp

/**
 * How far the wedge sits from the true corner.
 *
 * Far enough to be *inside* a round button. These buttons clip themselves to a
 * circle before anything is drawn on them, so a wedge in the true corner is cut
 * away almost entirely — what survives is a thin arc that reads as a rendering
 * fault rather than as a mark. Eight points puts the whole triangle within the
 * circle of a 44–46dp button, which is every button that has one.
 */
private val HINT_INSET = 8.dp

