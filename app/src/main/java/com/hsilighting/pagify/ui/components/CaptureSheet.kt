package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.Image
import com.hsilighting.pagify.core.CaptureFormat
import com.hsilighting.pagify.core.CaptureScale
import com.hsilighting.pagify.ui.reader.CapturePreview

/**
 * What was captured, and what to do with it.
 *
 * Shown after the region is taken rather than before, because every choice on it
 * is easier to make while looking at the result: whether the crop caught what was
 * wanted, whether it needs to be sharper, whether a photograph would be better as
 * a JPEG. Changing the scale or the format re-renders from the same crop, so
 * nothing has to be dragged out again.
 */
@Composable
fun CaptureSheet(
    preview: CapturePreview,
    isCapturing: Boolean,
    onScaleChange: (CaptureScale) -> Unit,
    onFormatChange: (CaptureFormat) -> Unit,
    onSaveToGallery: () -> Unit,
    onShare: () -> Unit,
    onCopy: () -> Unit,
    onDismiss: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp)
            .padding(bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Picture taken", style = MaterialTheme.typography.titleLarge)

        Box(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 320.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) {
            CapturedImage(preview.preview)
            // Over the image rather than replacing it: a re-render at a higher
            // scale takes a moment, and swapping the picture for a spinner makes
            // it look as though the capture was lost.
            if (isCapturing) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 320.dp)
                        .background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.35f)),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator()
                }
            }
        }

        Text(
            "Page ${preview.request.pageIndex + 1} · ${preview.request.scale.label} · " +
                "${preview.request.format.extension.uppercase()} · ${preview.sizeLabel}",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            CaptureScale.entries.forEach { scale ->
                FilterChip(
                    selected = preview.request.scale == scale,
                    onClick = { onScaleChange(scale) },
                    enabled = !isCapturing,
                    label = { Text(scale.label) },
                )
            }
            CaptureFormat.entries.forEach { format ->
                FilterChip(
                    selected = preview.request.format == format,
                    onClick = { onFormatChange(format) },
                    enabled = !isCapturing,
                    label = { Text(format.extension.uppercase()) },
                )
            }
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(
                onClick = onSaveToGallery,
                enabled = !isCapturing,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.Download, contentDescription = null, Modifier.size(18.dp))
                Text("  Save")
            }
            OutlinedButton(
                onClick = onShare,
                enabled = !isCapturing,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.Share, contentDescription = null, Modifier.size(18.dp))
                Text("  Share")
            }
            OutlinedButton(
                onClick = onCopy,
                enabled = !isCapturing,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.ContentCopy, contentDescription = null, Modifier.size(18.dp))
                Text("  Copy")
            }
        }

        TextButton(onClick = onDismiss, modifier = Modifier.align(Alignment.End)) {
            Text("Discard")
        }
    }
}

/**
 * `ContentScale.Fit`, so the whole capture is visible.
 *
 * Cropping the preview would hide exactly the thing the sheet exists to show —
 * whether the region caught what was meant.
 */
@Composable
private fun CapturedImage(image: ImageBitmap) {
    Image(
        bitmap = image,
        contentDescription = "The captured region",
        contentScale = ContentScale.Fit,
        modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp),
    )
}
