package com.hsilighting.pagify.ui.model

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Path
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CenterFocusStrong
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CropFree
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Gesture
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.ui.components.ToolButton
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.sqrt

/**
 * Taking a picture of the model.
 *
 * **Drawn again, not grabbed off the screen.** The same decision the PDF
 * reader's capture rests on: the picture is rendered afresh by the engine from
 * the camera the view is using, so it contains the part and nothing else — no
 * toolbar, no status bar, no notification that happened to arrive. Those pixels
 * are never in it because they are never drawn.
 *
 * It also means the picture need not be the size of the screen. A phone shows
 * about two megapixels; a capture is kept, sent on and looked at again later,
 * and is worth more than the display can show.
 */

/**
 * How large a capture to draw, in pixels.
 *
 * The view's own proportions are kept exactly, so the picture frames the part
 * the way the screen does. A capture that reframed it would be a different
 * picture from the one the user was looking at when they pressed the button.
 *
 * [mostPixels] is a ceiling on memory, not on ambition: the bitmap is four
 * bytes a pixel and is encoded from a second buffer, so an unbounded multiple
 * of a large screen is how this feature would become the reason the app is
 * killed. The one thing that outranks the ceiling is the view itself — a
 * capture coarser than what is already on screen would be worse than no
 * feature at all.
 */
fun captureSize(
    viewWidth: Int,
    viewHeight: Int,
    scale: Int,
    mostPixels: Int = MOST_CAPTURE_PIXELS,
): IntSize {
    if (viewWidth <= 0 || viewHeight <= 0) return IntSize.Zero

    val wide = viewWidth * max(1, scale)
    val high = viewHeight * max(1, scale)
    val pixels = wide.toLong() * high.toLong()
    if (pixels <= mostPixels) return IntSize(wide, high)

    // Rounded down, not to nearest: rounding both sides up puts the result back
    // over the ceiling it was brought under, by a few thousand pixels that no
    // arithmetic afterwards would notice.
    val shrink = sqrt(mostPixels.toDouble() / pixels.toDouble())
    return IntSize(
        max(viewWidth, (wide * shrink).toInt()),
        max(viewHeight, (high * shrink).toInt()),
    )
}

/**
 * Twelve megapixels: a 48 MB bitmap, held briefly while it is encoded.
 *
 * Chosen so it does not defeat the ordinary case. Twice a 1080 × 1920 phone is
 * already 8.3 megapixels, so a tighter ceiling would quietly turn every capture
 * on every phone into barely more than a screenshot — the ceiling is here for
 * the tablet at four times size, not for the case this feature exists to serve.
 */
const val MOST_CAPTURE_PIXELS: Int = 12_000_000

/**
 * Where a region dragged on screen lands in the re-rendered picture.
 *
 * Scaled by the same factor in both directions, and clipped to the picture:
 * a drag that ran off the edge of the view — which is how anyone selects
 * something against the border — would otherwise ask for pixels that were
 * never drawn.
 *
 * `null` when nothing usable is left, so a stray tap does not produce an empty
 * picture and a sheet on top of it.
 */
fun regionInCapture(box: Rect, viewWidth: Int, viewHeight: Int, whole: IntSize): IntRect? {
    if (viewWidth <= 0 || viewHeight <= 0 || whole.width <= 0 || whole.height <= 0) return null

    val across = whole.width.toFloat() / viewWidth.toFloat()
    val down = whole.height.toFloat() / viewHeight.toFloat()

    val left = (box.left * across).toInt().coerceIn(0, whole.width)
    val top = (box.top * down).toInt().coerceIn(0, whole.height)
    val right = (box.right * across).roundToInt().coerceIn(0, whole.width)
    val bottom = (box.bottom * down).roundToInt().coerceIn(0, whole.height)

    if (right - left < 1 || bottom - top < 1) return null
    return IntRect(left, top, right, bottom)
}

/**
 * Cut the region out, blanking anything the lasso left outside itself.
 *
 * The ring arrives in view pixels and is scaled and shifted into the cut
 * picture's own coordinates here, because that is the only place both are
 * known — passing it around already converted is how a mask ends up correct
 * at one zoom and wrong at every other.
 */
fun cutOut(whole: Bitmap, cut: IntRect, ring: List<Offset>, factor: Float): Bitmap {
    val region = Bitmap.createBitmap(whole, cut.left, cut.top, cut.width, cut.height)
    if (ring.size < 3) return region

    val path = Path()
    ring.forEachIndexed { at, point ->
        val x = point.x * factor - cut.left
        val y = point.y * factor - cut.top
        if (at == 0) path.moveTo(x, y) else path.lineTo(x, y)
    }
    path.close()

    // Drawn into a fresh bitmap through the ring rather than erased out of the
    // region: clipping away is not something a Bitmap canvas does reliably on
    // every Android version, and starting from blank cannot leave a fringe.
    val masked = Bitmap.createBitmap(region.width, region.height, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(masked)
    canvas.save()
    canvas.clipPath(path)
    canvas.drawBitmap(region, 0f, 0f, null)
    canvas.restore()
    return masked
}

/** What a capture is called once it is a file. */
fun captureFileName(model: String, stamp: String, format: CaptureFormat): String {
    // The model's own name first, because a folder of captures is sorted by
    // name and "2026-09-08 14-31-02.png" says nothing about which part it is.
    val stem = model.substringBeforeLast('.').ifBlank { "Model" }
    return "$stem $stamp.${format.extension}"
}

/**
 * The tool ribbon, along the bottom, as the reader has.
 *
 * **The same shape of control in both places.** On a page the snapshot tool is
 * a slot on a floating ribbon whose icon says which shape a drag will make, and
 * whose menu offers the other one. Putting the equivalent in the top bar of the
 * 3D viewer made it a different tool that happened to do the same thing —
 * somebody who knows one would still have to learn the other.
 *
 * Built from the reader's own `ToolButton` rather than from something that
 * looks like it, so the two cannot drift apart.
 */
@Composable
fun ModelRibbon(
    framing: Boolean,
    lasso: Boolean,
    onFrame: (Boolean) -> Unit,
    onLasso: (Boolean) -> Unit,
    onFit: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var showingShapes by remember { mutableStateOf(false) }

    Surface(
        modifier = modifier,
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
            Box {
                // The button means "snapshot" and the menu means "which shape",
                // exactly as on a page: the shape is only ever chosen after
                // deciding to take one, never before.
                ToolButton(
                    icon = if (framing && lasso) Icons.Filled.Gesture else Icons.Filled.CropFree,
                    label = "Snapshot",
                    selected = framing,
                    onClick = { showingShapes = true },
                    hasMore = true,
                )
                DropdownMenu(
                    expanded = showingShapes,
                    onDismissRequest = { showingShapes = false },
                ) {
                    ShapeChoice("Box", Icons.Filled.CropFree, framing && !lasso) {
                        showingShapes = false
                        onLasso(false)
                        // Choosing the shape already armed puts the tool away,
                        // so this stays a toggle rather than a one-way switch.
                        onFrame(!(framing && !lasso))
                    }
                    ShapeChoice("Ring", Icons.Filled.Gesture, framing && lasso) {
                        showingShapes = false
                        onLasso(true)
                        onFrame(!(framing && lasso))
                    }
                }
            }

            ToolButton(
                icon = Icons.Filled.CenterFocusStrong,
                label = "Fit",
                selected = false,
                onClick = onFit,
            )
        }
    }
}

@Composable
private fun ShapeChoice(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    armed: Boolean,
    onClick: () -> Unit,
) {
    DropdownMenuItem(
        text = { Text(label) },
        leadingIcon = { Icon(icon, contentDescription = null) },
        trailingIcon = {
            if (armed) Icon(Icons.Filled.Check, contentDescription = "Armed")
        },
        onClick = onClick,
    )
}

/**
 * The picture, and what can be done with it.
 *
 * Shown rather than saved straight away. A capture of a 3D view is a framing
 * decision — the part is where the reader turned it to — so seeing the result
 * before choosing what to do with it is the difference between one press and
 * three tries.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CaptureSheet(
    picture: Bitmap,
    format: CaptureFormat,
    onFormat: (CaptureFormat) -> Unit,
    onSave: () -> Unit,
    onShare: () -> Unit,
    onCopy: () -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(
                "Picture of the model",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )

            Image(
                bitmap = picture.asImageBitmap(),
                contentDescription = "The captured picture",
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 260.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(Color(0xFF14171C)),
                contentScale = ContentScale.Fit,
            )

            Text(
                "%,d × %,d pixels".format(picture.width, picture.height),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CaptureFormat.entries.forEach { choice ->
                    FilterChip(
                        selected = choice == format,
                        onClick = { onFormat(choice) },
                        label = { Text(choice.extension.uppercase()) },
                    )
                }
            }

            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Action(Icons.Filled.Download, "Save", onSave)
                Action(Icons.Filled.Share, "Share", onShare)
                Action(Icons.Filled.ContentCopy, "Copy", onCopy)
            }
        }
    }
}

@Composable
private fun Action(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    TextButton(onClick = onClick) {
        Icon(icon, contentDescription = null, modifier = Modifier.padding(end = 6.dp))
        Text(label)
    }
}
