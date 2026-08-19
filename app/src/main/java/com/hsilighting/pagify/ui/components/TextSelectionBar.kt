package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Highlight
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * What to do with the text that is selected.
 *
 * Anchored above the tool ribbon rather than floating over the selection, which
 * is where a context menu would normally go. A menu at the finger covers the
 * words it belongs to — and this selection can run over several lines, so there
 * is no "beside it" that is not on top of something.
 *
 * The count is there because a selection dragged across a column break can pick
 * up far more than it appears to, and a number is the cheapest way to notice
 * before it lands on the clipboard.
 */
@Composable
fun TextSelectionBar(
    characters: Int,
    onCopy: () -> Unit,
    onHighlight: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "$characters selected",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.padding(start = 8.dp, end = 4.dp),
            )
            TextButton(onClick = onCopy) {
                Icon(Icons.Filled.ContentCopy, contentDescription = null, Modifier.size(18.dp))
                Text("  Copy")
            }
            TextButton(onClick = onHighlight) {
                Icon(Icons.Filled.Highlight, contentDescription = null, Modifier.size(18.dp))
                Text("  Highlight")
            }
            TextButton(onClick = onDismiss) {
                Icon(Icons.Filled.Close, contentDescription = "Clear the selection")
            }
        }
    }
}
