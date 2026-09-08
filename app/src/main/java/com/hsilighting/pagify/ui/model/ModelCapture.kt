package com.hsilighting.pagify.ui.model

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.CaptureFormat
import kotlin.math.max
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

/** What a capture is called once it is a file. */
fun captureFileName(model: String, stamp: String, format: CaptureFormat): String {
    // The model's own name first, because a folder of captures is sorted by
    // name and "2026-09-08 14-31-02.png" says nothing about which part it is.
    val stem = model.substringBeforeLast('.').ifBlank { "Model" }
    return "$stem $stamp.${format.extension}"
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
