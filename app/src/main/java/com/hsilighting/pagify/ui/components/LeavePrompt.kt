package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.ui.reader.LeaveIntent

/**
 * "You have unsaved marks."
 *
 * Three answers and a way out. Exit leaves them behind, Save writes them where
 * they came from, Save as writes them somewhere new — and the cross closes the
 * question without answering it, which is what a reader who pressed back by
 * accident actually wants.
 *
 * The cross rather than a Cancel button because the two are not the same idea:
 * the buttons are things to do about the marks, and closing the dialog is not one
 * of them. Sitting apart, in the corner, it reads as the dialog's own control.
 */
@Composable
fun LeavePrompt(
    intent: LeaveIntent,
    onSave: () -> Unit,
    onSaveAs: () -> Unit,
    onExit: () -> Unit,
    onClose: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onClose,
        title = {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Save your changes?")
                IconButton(onClick = onClose, modifier = Modifier.size(32.dp)) {
                    Icon(
                        Icons.Filled.Close,
                        contentDescription = "Close",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        text = {
            Text(
                when (intent) {
                    LeaveIntent.Library -> "This document has marks that have not been saved."
                    LeaveIntent.AnotherDocument ->
                        "This document has marks that have not been saved. " +
                            "Opening another will leave them behind."
                },
            )
        },
        confirmButton = { TextButton(onClick = onSave) { Text("Save") } },
        // Both remaining answers share the dismiss slot. Exit sits furthest from
        // Save so the two opposite meanings are not neighbours under a thumb.
        dismissButton = {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                TextButton(onClick = onExit) { Text("Exit") }
                TextButton(onClick = onSaveAs) { Text("Save as") }
            }
        },
    )
}
