package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin

/**
 * Pick any colour, from a wheel.
 *
 * A wheel rather than three sliders because the question being asked is "which
 * colour", not "how much red": hue around, saturation outward, and one slider for
 * brightness. Someone reaching for this has a colour in mind and wants to point
 * at it.
 *
 * The palette beside it covers the common cases; this is for the ones it does
 * not, which is why it is a dialog rather than another row of controls always on
 * screen.
 */
@Composable
fun ColourWheelDialog(
    initial: Long,
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
) {
    val start = remember(initial) { hsvOf(initial) }
    var hue by remember { mutableFloatStateOf(start[0]) }
    var saturation by remember { mutableFloatStateOf(start[1]) }
    var value by remember { mutableFloatStateOf(start[2]) }

    val picked = hsvColour(hue, saturation, value)

    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton(onClick = { onPick(picked.toArgbLong()) }) { Text("Use this colour") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
        title = { Text("Pick a colour") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .aspectRatio(1f)
                        .padding(horizontal = 12.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Wheel(
                        hue = hue,
                        saturation = saturation,
                        value = value,
                        onChange = { h, s ->
                            hue = h
                            saturation = s
                        },
                    )
                }

                BrightnessSlider(
                    hue = hue,
                    saturation = saturation,
                    value = value,
                    onChange = { value = it },
                )

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier
                            .size(36.dp)
                            .background(picked, CircleShape),
                    )
                    Text(
                        // The hex is not decoration: it is how a colour gets
                        // matched to one already in a document, or written down.
                        picked.toHex(),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
    )
}

/**
 * Hue around, saturation outward.
 *
 * Drawn as spokes rather than a shader: a sweep gradient gives the hues but not
 * the saturation falloff, and a per-pixel bitmap would be rebuilt on every
 * recomposition. A few hundred anti-aliased wedges are indistinguishable from a
 * continuous wheel at this size and cost nothing to redraw.
 */
@Composable
private fun Wheel(
    hue: Float,
    saturation: Float,
    value: Float,
    onChange: (hue: Float, saturation: Float) -> Unit,
) {
    Canvas(
        Modifier
            .fillMaxWidth()
            .aspectRatio(1f)
            .pointerInput(Unit) {
                // Tap and drag both, because both are natural here: a tap to jump
                // to a colour, a drag to hunt for one.
                detectTapGestures { position -> report(position, size.width, size.height, onChange) }
            }
            .pointerInput(Unit) {
                detectDragGestures { change, _ ->
                    change.consume()
                    report(change.position, size.width, size.height, onChange)
                }
            },
    ) {
        val radius = size.minDimension / 2f
        val centre = Offset(size.width / 2f, size.height / 2f)

        for (step in 0 until WEDGES) {
            val from = step * 360f / WEDGES
            drawArc(
                brush = Brush.radialGradient(
                    colors = listOf(hsvColour(from, 0f, value), hsvColour(from, 1f, value)),
                    center = centre,
                    radius = radius,
                ),
                startAngle = from - 0.5f,
                // A hair of overlap, or the seams between wedges show as spokes.
                sweepAngle = 360f / WEDGES + 1f,
                useCenter = true,
            )
        }

        // Where the current colour sits, so the wheel shows a state rather than
        // just offering a choice.
        val marker = centre + Offset(
            cos(Math.toRadians(hue.toDouble())).toFloat() * saturation * radius,
            sin(Math.toRadians(hue.toDouble())).toFloat() * saturation * radius,
        )
        drawCircle(Color.White, radius = MARKER_RADIUS_PX, center = marker, style = Stroke(4f))
        drawCircle(Color.Black, radius = MARKER_RADIUS_PX, center = marker, style = Stroke(2f))
    }
}

private fun report(
    position: Offset,
    width: Int,
    height: Int,
    onChange: (Float, Float) -> Unit,
) {
    val centre = Offset(width / 2f, height / 2f)
    val radius = minOf(width, height) / 2f
    val delta = position - centre
    val distance = hypot(delta.x, delta.y)

    val hue = (Math.toDegrees(atan2(delta.y, delta.x).toDouble()).toFloat() + 360f) % 360f
    // Clamped rather than ignored past the edge: a finger that slides off the
    // wheel while dragging should hold the outer colour, not drop the gesture.
    onChange(hue, (distance / radius).coerceIn(0f, 1f))
}

/** Black to the full-strength colour, which is what "brightness" means here. */
@Composable
private fun BrightnessSlider(
    hue: Float,
    saturation: Float,
    value: Float,
    onChange: (Float) -> Unit,
) {
    Box(
        Modifier
            .fillMaxWidth()
            .height(36.dp)
            .padding(horizontal = 12.dp),
    ) {
        Canvas(
            Modifier
                .fillMaxWidth()
                .height(36.dp)
                .pointerInput(Unit) {
                    detectTapGestures { onChange((it.x / size.width).coerceIn(0f, 1f)) }
                }
                .pointerInput(Unit) {
                    detectDragGestures { change, _ ->
                        change.consume()
                        onChange((change.position.x / size.width).coerceIn(0f, 1f))
                    }
                },
        ) {
            drawTrack(hue, saturation)
            val x = value * size.width
            drawCircle(Color.White, radius = 12f, center = Offset(x, size.height / 2f))
            drawCircle(
                Color.Black.copy(alpha = 0.4f),
                radius = 12f,
                center = Offset(x, size.height / 2f),
                style = Stroke(2f),
            )
        }
    }
}

private fun DrawScope.drawTrack(hue: Float, saturation: Float) {
    drawRoundRect(
        brush = Brush.horizontalGradient(
            listOf(hsvColour(hue, saturation, 0f), hsvColour(hue, saturation, 1f)),
        ),
        cornerRadius = androidx.compose.ui.geometry.CornerRadius(18f, 18f),
    )
}

/** HSV to a colour. `hue` in degrees, the rest 0..1. */
private fun hsvColour(hue: Float, saturation: Float, value: Float): Color =
    Color(android.graphics.Color.HSVToColor(floatArrayOf(hue % 360f, saturation, value)))

private fun hsvOf(argb: Long): FloatArray {
    val hsv = FloatArray(3)
    android.graphics.Color.colorToHSV(argb.toInt(), hsv)
    return hsv
}

private fun Color.toArgbLong(): Long =
    (0xFFL shl 24) or
        ((red * 255f).toLong() shl 16) or
        ((green * 255f).toLong() shl 8) or
        (blue * 255f).toLong()

private fun Color.toHex(): String = "#%06X".format(toArgbLong() and 0xFFFFFFL)

/**
 * Wedges the wheel is drawn from.
 *
 * 180 is two degrees each, which is past the point where a seam is visible at any
 * size this dialog can be.
 */
private const val WEDGES = 180

private const val MARKER_RADIUS_PX = 14f

/**
 * The way out of a fixed palette, and into [ColourWheelDialog].
 *
 * Shows the wheel it opens until a colour has been picked from it, and the colour
 * itself afterwards — otherwise choosing a custom colour makes the selection ring
 * vanish from the row and nothing on screen says what is being drawn with.
 *
 * Shared by the reader's tool band and the screenshot editor's, because "any
 * colour" is the same offer in both and two drawings of it would drift apart.
 */
@Composable
fun CustomColourSwatch(
    current: Long,
    isCustom: Boolean,
    onClick: () -> Unit,
    size: Dp = 26.dp,
) {
    Box(
        modifier = Modifier
            .size(size)
            .border(
                width = if (isCustom) 3.dp else 1.dp,
                color = if (isCustom) {
                    MaterialTheme.colorScheme.onSurface
                } else {
                    MaterialTheme.colorScheme.outlineVariant
                },
                shape = CircleShape,
            )
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(size - 4.dp)) {
            if (isCustom) {
                drawCircle(Color(current))
            } else {
                drawCircle(
                    brush = Brush.sweepGradient(
                        listOf(
                            Color.Red,
                            Color.Yellow,
                            Color.Green,
                            Color.Cyan,
                            Color.Blue,
                            Color.Magenta,
                            Color.Red,
                        ),
                    ),
                )
            }
        }
    }
}
